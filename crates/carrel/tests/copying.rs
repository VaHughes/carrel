//! Copying things out of a document in the shape they get pasted in.
//!
//! The reader this is for is talking to an agent in the next pane. What they
//! copy is an instruction in the making — "look at PLAN.md:42", a quoted
//! passage with its source, their notes — so each of these has to produce
//! text that means the same thing on the other side of the paste.

use carrel::action::{Action, Zone};
use carrel::app::{App, Outcome, update};
use carrel_core::{BlockIdx, Document, NodeKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const SRC: &str = "# Rollout\n\nAn opening paragraph that is long enough to wrap onto a second \
row when the window is narrow, and then some.\n\n## Step 3: Do it\n\nThe first line of the step.\n\
The second line of the step.\n\n```sh\nmake build\nmake test\n```\n\nSome prose between.\n\n\
```rust\nlet x = 1;\n```\n\n## Step 3: Do it\n\nA second section with the same name.\n";

fn reader(cols: u16, rows: u16) -> App {
    let mut app = App::new("PLAN.md".into(), Document::parse(SRC), cols, rows);
    app.file = Some("/work/repo/docs/PLAN.md".into());
    app.launch_dir = Some("/work/repo".into());
    app.on_resize(cols, rows);
    app
}

fn byte_of(app: &App, needle: &str) -> u32 {
    u32::try_from(app.doc.text.find(needle).expect("in the document")).unwrap()
}

#[test]
fn a_reference_is_the_path_from_where_you_ran_carrel_and_the_files_own_line() {
    // Narrow, so the opening paragraph wraps: the line copied is the line
    // OF THE FILE, which no visual row number would give.
    let mut app = reader(30, 40);
    let at = byte_of(&app, "The second line");
    assert_eq!(update(&mut app, Action::CopyRef(at)), Outcome::Redraw);
    assert_eq!(app.clipboard.as_deref(), Some("docs/PLAN.md:8"));
    assert_eq!(SRC.lines().nth(7), Some("The second line of the step."));
    assert!(app.note.as_deref().unwrap().contains("docs/PLAN.md:8"));
}

#[test]
fn a_file_outside_the_launch_folder_is_named_in_full() {
    let mut app = reader(80, 40);
    app.launch_dir = Some("/somewhere/else".into());
    update(&mut app, Action::CopyRef(0));
    assert_eq!(app.clipboard.as_deref(), Some("/work/repo/docs/PLAN.md:1"));
}

#[test]
fn a_document_with_no_file_has_nothing_to_point_at() {
    let mut app = App::new("(stdin)".into(), Document::parse(SRC), 80, 40);
    for action in [Action::CopyRef(0), Action::CopySection(40)] {
        update(&mut app, action);
        assert_eq!(
            app.clipboard, None,
            "{action:?} must not copy a made-up path"
        );
        assert!(app.note.as_deref().unwrap().contains("no file"));
    }
}

#[test]
fn a_diff_is_referred_to_without_a_line() {
    // Its lines are the lines of the markdown carrel generated, not the file's.
    let mut app = reader(80, 40);
    app.diff_ok = true;
    update(&mut app, Action::CopyRef(30));
    assert_eq!(app.clipboard.as_deref(), Some("docs/PLAN.md"));
}

#[test]
fn a_section_link_names_the_innermost_heading_and_resolves_back_to_it() {
    let mut app = reader(80, 60);
    let first = byte_of(&app, "The first line");
    update(&mut app, Action::CopySection(first));
    assert_eq!(
        app.clipboard.take().as_deref(),
        Some("docs/PLAN.md#step-3-do-it")
    );

    // The second heading of the same name is a different link.
    let second = byte_of(&app, "A second section");
    update(&mut app, Action::CopySection(second));
    let link = app.clipboard.take().unwrap();
    assert_eq!(link, "docs/PLAN.md#step-3-do-it-1");
    let fragment = link.split_once('#').unwrap().1;
    let target = app.doc.fragment_target(fragment).unwrap();
    assert!(
        target > first && target < second,
        "the link must land on the heading above what was clicked"
    );
}

#[test]
fn above_the_first_heading_there_is_no_section_to_link() {
    let mut app = App::new("n.md".into(), Document::parse("intro\n\n# Later\n"), 80, 20);
    app.file = Some("/w/n.md".into());
    update(&mut app, Action::CopySection(0));
    assert_eq!(app.clipboard, None);
    assert!(app.note.as_deref().unwrap().contains("no heading"));
}

#[test]
fn a_quote_carries_its_text_line_for_line_and_says_where_it_came_from() {
    let mut app = reader(80, 60);
    let start = byte_of(&app, "The first line");
    let end = byte_of(&app, "The second line") + 28;
    app.selection = Some(start..end);
    update(&mut app, Action::CopyQuote);
    assert_eq!(
        app.clipboard.as_deref(),
        Some(
            "> The first line of the step. The second line of the step.\n\n\
             \u{2014} docs/PLAN.md:7 (Rollout \u{203a} Step 3: Do it)\n"
        )
    );
}

#[test]
fn a_quote_needs_a_selection_and_a_pathless_one_is_signed_with_its_label() {
    let mut app = App::new(
        "(stdin)".into(),
        Document::parse("# T\n\nsome text\n"),
        80,
        20,
    );
    update(&mut app, Action::CopyQuote);
    assert_eq!(app.clipboard, None);
    assert!(app.note.as_deref().unwrap().contains("select"));

    let at = u32::try_from(app.doc.text.find("some").unwrap()).unwrap();
    app.selection = Some(at..at + 9);
    update(&mut app, Action::CopyQuote);
    assert_eq!(
        app.clipboard.as_deref(),
        Some("> some text\n\n\u{2014} (stdin) (T)\n")
    );
}

fn code_blocks(app: &App) -> Vec<BlockIdx> {
    (0..app.doc.block_count())
        .map(|b| BlockIdx(b as u32))
        .filter(|b| matches!(app.doc.node_for_block(*b).kind, NodeKind::CodeBlock { .. }))
        .collect()
}

#[test]
fn a_blocks_own_chip_copies_that_block() {
    let mut app = reader(80, 60);
    let blocks = code_blocks(&app);
    assert_eq!(blocks.len(), 2);
    // The SECOND one, with nothing focused: `y` would have taken the first.
    update(&mut app, Action::YankBlockAt(blocks[1]));
    assert_eq!(app.clipboard.take().as_deref(), Some("let x = 1;\n"));
    assert_eq!(
        app.code_focus,
        Some(blocks[1]),
        "and it is the focused block now"
    );

    // A chip from a frame the document has outlived does nothing.
    assert_eq!(
        update(&mut app, Action::YankBlockAt(BlockIdx(999))),
        Outcome::Idle
    );
    assert_eq!(
        update(&mut app, Action::YankBlockAt(BlockIdx(0))),
        Outcome::Idle
    );
    assert_eq!(app.clipboard, None, "a heading is not a code block");
}

/// The chip is painted where the target is registered, on a row that holds
/// no text — for every code block on screen, not only a focused one.
#[test]
fn every_code_block_on_screen_offers_to_be_copied() {
    let (cols, rows) = (80u16, 60u16);
    let app = reader(cols, rows);
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let buf = term.backend().buffer().clone();

    let chips: Vec<(BlockIdx, Zone)> = painted
        .targets
        .as_slice()
        .iter()
        .filter_map(|t| match t.action {
            Action::YankBlockAt(b) => Some((b, t.zone)),
            _ => None,
        })
        .collect();
    assert_eq!(
        chips.iter().map(|c| c.0).collect::<Vec<_>>(),
        code_blocks(&app),
        "one chip per code block"
    );
    for (block, zone) in chips {
        let label: String = (zone.x..zone.x + zone.w)
            .map(|x| buf[(x, zone.y)].symbol())
            .collect();
        assert_eq!(label, "copy", "{block:?}: the target covers its own word");
        // Nothing else on that row: the chip sits on the block's gap.
        let rest: String = (0..zone.x).map(|x| buf[(x, zone.y)].symbol()).collect();
        assert_eq!(
            rest.trim(),
            "",
            "{block:?}: the chip shares a row with text"
        );
    }
}
