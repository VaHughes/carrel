//! Empty files, or files temporarily emptied during a rewrite, remain usable.
use carrel::action::Action;
use carrel::app::{App, Outcome, update};
use carrel_core::{BlockIdx, Document};
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn document_actions_are_safe_without_a_visible_block() {
    for source in [
        "",
        " \n\t",
        "<!-- only a comment -->",
        "[ref]: elsewhere.md\n",
    ] {
        for action in [
            Action::LinkFollow,
            Action::MarkToggle,
            Action::FootnoteJump,
            Action::SelectBlock(0),
            Action::FoldAt(0),
            Action::OutlineJumpTo(BlockIdx(0)),
            Action::TagOpen(0),
            Action::BlockStep(1),
            Action::CodeStep(1),
            Action::TaskOpen,
            Action::TaskStep(1),
            Action::ChangeStep(1),
            Action::YankBlock,
            Action::FoldToggle,
            Action::FoldAll,
            Action::UnfoldAll,
            Action::MarkNext,
            Action::MarkListToggle,
            Action::ImageOpen(None),
        ] {
            let mut app = App::new("empty.md".into(), Document::parse(source), 80, 24);
            assert_eq!(app.doc.block_count(), 0, "{source:?}");
            assert_ne!(update(&mut app, action), Outcome::Quit, "{action:?}");
            assert_eq!(app.view.anchor, 0);
            assert_eq!(app.clipboard, None, "{action:?}");
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|f| carrel::render::draw(f, &app)).unwrap();
        }
    }
}
