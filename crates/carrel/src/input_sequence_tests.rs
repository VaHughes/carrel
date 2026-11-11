//! Mixed reader input, through the binary's real keyboard dispatcher.
use super::*;
use carrel::action::Action;
use proptest::prelude::*;
use ratatui::{Terminal, backend::TestBackend};

const SOURCE: &str = "# Start\n\nCafé 👩‍💻 text and [section](#second).[^a]\n\n\
    | Name | Value |\n| --- | --- |\n| α | a long table cell |\n\n\
    ## Second\n\n- [ ] Task\n\n```rust\nfn main() {}\n```\n\n\
    <details><summary>Details</summary>\n\nHidden text.\n\n</details>\n\n\
    [^a]: Footnote with text that wraps in a small window.\n";

fn key(index: u16) -> KeyEvent {
    let codes = [
        KeyCode::Char('/'),
        KeyCode::Char('?'),
        KeyCode::Char('h'),
        KeyCode::Char('o'),
        KeyCode::Char(','),
        KeyCode::Char('I'),
        KeyCode::Char('m'),
        KeyCode::Char('"'),
        KeyCode::Char('n'),
        KeyCode::Char('N'),
        KeyCode::Char('z'),
        KeyCode::Char('a'),
        KeyCode::Char('v'),
        KeyCode::Char('V'),
        KeyCode::Char('t'),
        KeyCode::Char('S'),
        KeyCode::Char('B'),
        KeyCode::Char('H'),
        KeyCode::Char('r'),
        KeyCode::Char('F'),
        KeyCode::Char('A'),
        KeyCode::Char('['),
        KeyCode::Char(']'),
        KeyCode::Char('0'),
        KeyCode::Char('9'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char('%'),
        KeyCode::Tab,
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Backspace,
        KeyCode::F(3),
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
    ];
    KeyEvent::new(codes[usize::from(index) % codes.len()], KeyModifiers::NONE)
}

proptest! {
    #[test]
    fn mixed_input_keeps_positions_valid_and_only_one_pane(
        events in prop::collection::vec((0u8..12, 0u16..128, 0u16..48, "[a-zé界👩\u{200d}\u{301}\\r\\n]{0,20}"), 1..80)
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let other = scratch.path().join("other.md");
        std::fs::write(&other, SOURCE).unwrap();
        let source = format!("{SOURCE}\n[other]({})\n", carrel::links::encode_target(other.to_str().unwrap()));
        let mut app = App::new("(stdin)".into(), carrel_core::Document::parse(&source), 80, 24);
        app.streaming = true;
        app.piped = Some(source);
        app.library_root = Some(scratch.path().into());
        let mut keys = Keys::new();
        let mut size = (80, 24);
        let mut painted = Painted::default();
        let mut pointer = Pointer::default();
        for (kind, a, b, text) in events {
            match kind {
                0 => {
                    size = (a, b);
                    let ranges = app.matches.as_ref().map(|m| m.ranges.clone());
                    let current = app.matches.as_ref().and_then(|m| m.current);
                    app.on_resize(a, b);
                    pointer.forget_geometry();
                    prop_assert_eq!(app.matches.as_ref().map(|m| &m.ranges), ranges.as_ref());
                    prop_assert_eq!(app.matches.as_ref().and_then(|m| m.current), current);
                }
                1 => { carrel::app::paste_input(&mut app, &text); }
                8 => {
                    let replacement = match a % 5 {
                        0 => String::new(),
                        1 => format!("# {text}\n\n界 👩‍💻"),
                        2 => format!("{SOURCE}\n\n{text}"),
                        3 => format!("{text}\n\n{SOURCE}"),
                        _ => format!("- {text}\n- [x] Done\n\n```rust\nlet x = 1;\n```"),
                    };
                    if let Some(path) = &app.file {
                        std::fs::write(path, &replacement).unwrap();
                        app.reload_from(&replacement);
                    } else if app.showing_pipe() {
                        app.piped = Some(replacement.clone());
                        app.reload_from(&replacement);
                    }
                }
                11 => {
                    let action = match a % 4 {
                        0 => Action::WelcomeOpen,
                        1 => Action::Back,
                        2 => Action::Forward,
                        _ => Action::LinkOpen(u32::from(b) % (app.doc.links.len() as u32).max(1)),
                    };
                    update(&mut app, action);
                }
                2..=4 | 9..=10 => {
                    let kind = match kind {
                        2 => MouseEventKind::Down(MouseButton::Left),
                        3 => MouseEventKind::Drag(MouseButton::Left),
                        9 => MouseEventKind::Up(MouseButton::Left),
                        10 => MouseEventKind::Down(MouseButton::Right),
                        _ => if a % 2 == 0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown },
                    };
                    let event = MouseEvent { kind, column: a, row: b, modifiers: KeyModifiers::NONE };
                    if let Some(action) = mouse_action(event, &app, &painted.targets, &mut pointer)
                        && update(&mut app, action) == Outcome::Quit
                    {
                        break;
                    }
                }
                _ => {
                    if let Some(action) = key_action(&mut keys, &app, key(a))
                        && update(&mut app, action) == Outcome::Quit
                    {
                        break;
                    }
                }
            }
            prop_assert!(!(app.following && app.auto_read));
            prop_assert!(app.view.anchor as usize <= app.doc.text.len());
            prop_assert!(app.doc.text.is_char_boundary(app.view.anchor as usize));
            if let carrel::app::Mode::Search { saved, .. } = &app.mode {
                prop_assert!(*saved as usize <= app.doc.text.len());
                prop_assert!(app.doc.text.is_char_boundary(*saved as usize));
            }
            if let Some(selection) = &app.selection {
                prop_assert!(selection.start <= selection.end && selection.end as usize <= app.doc.text.len());
                prop_assert!(app.doc.text.is_char_boundary(selection.start as usize));
                prop_assert!(app.doc.text.is_char_boundary(selection.end as usize));
            }
            if let Some(draft) = &app.notes.draft {
                use unicode_segmentation::UnicodeSegmentation;
                prop_assert!(draft.cursor == draft.text.len()
                    || draft.text.grapheme_indices(true).any(|(at, _)| at == draft.cursor));
            }
            for mark in &app.marks {
                prop_assert!((*mark as usize) < app.doc.text.len());
                prop_assert!(app.doc.text.is_char_boundary(*mark as usize));
            }
            for note in &app.notes.entries {
                if let Some(range) = &note.range {
                    prop_assert!(range.start < range.end && range.end as usize <= app.doc.text.len());
                    prop_assert!(app.doc.text.is_char_boundary(range.start as usize));
                    prop_assert!(app.doc.text.is_char_boundary(range.end as usize));
                }
            }
            for id in &app.folded {
                prop_assert!(app.doc.nodes.iter().any(|node| node.id == *id && matches!(node.kind, carrel_core::NodeKind::Heading { .. })), "collapsed heading missing from document");
            }
            if let Some(matches) = &app.matches {
                prop_assert!(matches.current.is_none_or(|index| index < matches.len()));
                for range in &matches.ranges {
                    prop_assert!(range.start < range.end && range.end as usize <= app.doc.text.len());
                    prop_assert!(app.doc.text.is_char_boundary(range.start as usize));
                    prop_assert!(app.doc.text.is_char_boundary(range.end as usize));
                }
            }
            let panes = [
                app.lightbox.is_some(), app.notes.draft.is_some() || app.notes.pane.is_some(),
                app.menu.is_some(), app.help.is_some(), app.backlinks.is_some(),
                app.forward.is_some(), app.settings.is_some(), app.mark_list.is_some(),
                app.outline.is_some(), app.info, app.searching(),
            ].into_iter().filter(|shown| *shown).count();
            prop_assert!(panes <= 1, "{panes} panes survived input");
            // Paint each intermediate state, including zero-sized terminals.
            let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
            terminal.draw(|f| {
                render::draw_full(f, &app, &mut painted, &mut HashMap::new());
            }).unwrap();
            for target in painted.targets.as_slice() {
                prop_assert!(target.zone.x.saturating_add(target.zone.w) <= size.0);
                prop_assert!(target.zone.y.saturating_add(target.zone.h) <= size.1);
            }
        }
    }
}
