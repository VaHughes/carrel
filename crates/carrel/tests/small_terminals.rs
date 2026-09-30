//! Small panes must remain readable and every modal selection must stay visible.
use carrel::action::Action;
use carrel::app::{App, update};
use carrel::render::{Painted, draw_full};
use carrel_core::Document;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn app(cols: u16, rows: u16) -> App {
    App::new(
        "a-very-long-document-name.md".into(),
        Document::parse("# Heading\n\nalpha beta gamma delta epsilon\n"),
        cols,
        rows,
    )
}
fn frame(app: &App) -> (Buffer, Painted) {
    let mut terminal = Terminal::new(TestBackend::new(app.cols, app.rows)).unwrap();
    let mut painted = Painted::default();
    terminal
        .draw(|f| draw_full(f, app, &mut painted, &mut std::collections::HashMap::new()))
        .unwrap();
    for target in painted.targets.as_slice() {
        let z = target.zone;
        assert!(
            z.x + z.w <= app.cols && z.y + z.h <= app.rows,
            "target {:?} outside {}x{}",
            target,
            app.cols,
            app.rows
        );
    }
    (terminal.backend().buffer().clone(), painted)
}
fn text(buf: &Buffer) -> String {
    buf.content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}
fn has_action(painted: &Painted, cols: u16, rows: u16, action: Action) -> bool {
    (0..rows).any(|y| {
        (0..cols).any(|x| {
            painted
                .targets
                .hit(x, y)
                .is_some_and(|t| t.action == action)
        })
    })
}

#[test]
fn short_reader_spends_space_on_text_and_restores_preferences() {
    let mut app = app(40, 6);
    assert!(app.text_h() >= 4, "only {} reading rows", app.text_h());
    assert!(app.text_w() >= 37);
    assert!(app.hints && app.breadcrumb);
    assert!(!app.band());
    let (buf, _) = frame(&app);
    assert!(text(&buf).contains("Heading"));
    app.on_resize(80, 24);
    assert!(app.band());
    assert!(app.hints && app.breadcrumb);
    assert_eq!(app.text_y(), 2);
}

#[test]
fn narrow_help_still_explains_the_keys() {
    let mut app = app(24, 8);
    update(&mut app, Action::HelpToggle);
    app.help.as_mut().unwrap().filter = "quit".into();
    let (buf, _) = frame(&app);
    assert!(
        (1..7).any(|y| (0..24)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .contains("close file")),
        "{}",
        text(&buf)
    );
}

#[test]
fn settings_selection_is_visible_and_clickable_in_a_short_pane() {
    let mut app = app(32, 8);
    update(&mut app, Action::SettingsToggle);
    let last = carrel::app::settings_rows(&app).len() - 1;
    app.settings = Some(last);
    let (_, painted) = frame(&app);
    assert!(has_action(
        &painted,
        app.cols,
        app.rows,
        Action::SettingsPickAt(last as u32)
    ));
}

#[test]
fn short_menu_scrolls_to_the_selected_item() {
    let mut app = app(32, 8);
    update(
        &mut app,
        Action::MenuOpen {
            at: (31, 7),
            byte: None,
        },
    );
    let menu = app.menu.as_mut().unwrap();
    let last = menu
        .items
        .iter()
        .rposition(carrel::menu::Item::pickable)
        .unwrap();
    menu.selected = Some(last);
    let (_, painted) = frame(&app);
    assert!(has_action(
        &painted,
        app.cols,
        app.rows,
        Action::MenuPick(last as u32)
    ));
}

#[test]
fn narrow_reader_keeps_a_visible_menu_launcher() {
    let app = app(24, 8);
    let (buf, painted) = frame(&app);
    assert!(text(&buf).contains('≡'));
    assert!((0..app.rows).any(|y| (0..app.cols).any(|x| {
        painted
            .targets
            .hit(x, y)
            .is_some_and(|t| matches!(t.action, Action::MenuOpen { .. }))
    })));
}

#[test]
fn resizing_every_pane_to_tiny_dimensions_is_safe() {
    for (cols, rows) in [
        (0, 0),
        (1, 1),
        (2, 2),
        (4, 3),
        (10, 4),
        (12, 3),
        (20, 5),
        (40, 8),
        (59, 11),
        (60, 12),
    ] {
        for action in [
            Action::HelpToggle,
            Action::SettingsToggle,
            Action::OutlineToggle,
            Action::MarkListToggle,
            Action::ForwardToggle,
            Action::InfoToggle,
        ] {
            let mut app = app(80, 24);
            update(&mut app, action);
            app.on_resize(cols, rows);
            let _ = frame(&app);
        }
        let mut home = App::new_home("/tmp".into(), vec![], 80, 24);
        home.on_resize(cols, rows);
        let _ = frame(&home);
        update(&mut home, Action::PickerOpen);
        let _ = frame(&home);
    }
}

#[test]
fn menu_hover_does_not_move_the_row_under_the_pointer() {
    let mut app = app(32, 8);
    update(
        &mut app,
        Action::MenuOpen {
            at: (31, 7),
            byte: None,
        },
    );
    for _ in 0..30 {
        update(&mut app, Action::MenuMove(1));
    }
    let (_, painted) = frame(&app);
    let hit = (1..7)
        .flat_map(|y| (0..32).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            let action = painted.targets.hit(x, y)?.action;
            if let Action::MenuPick(i) = action {
                Some((x, y, i))
            } else {
                None
            }
        })
        .unwrap();
    update(&mut app, Action::MenuHover(hit.2));
    let (_, after) = frame(&app);
    assert_eq!(
        after.targets.hit(hit.0, hit.1).unwrap().action,
        Action::MenuPick(hit.2)
    );
}

#[test]
fn info_scroll_reaches_the_last_field_without_moving_the_document() {
    use carrel::action::Span;
    let mut app = app(24, 6);
    let anchor = app.view.anchor;
    update(&mut app, Action::InfoToggle);
    let last_value = app.info_rows().last().unwrap().1.clone();
    for _ in 0..100 {
        update(&mut app, Action::Scroll(Span::Line, 1));
    }
    let (buf, _) = frame(&app);
    assert!(text(&buf).contains(&last_value), "{}", text(&buf));
    assert_eq!(app.view.anchor, anchor);
}

#[test]
fn compact_home_keeps_files_visible_and_resume_clicks_honest() {
    use carrel::{home::Resume, scan::Entry};
    let mut app = App::new_home(
        "/docs".into(),
        vec![Entry {
            path: "/docs/visible.md".into(),
            mtime: std::time::UNIX_EPOCH,
        }],
        24,
        5,
    );
    app.home_mut().unwrap().resume = vec![Resume {
        path: "/docs/recent.md".into(),
        percent: 50,
        minutes_left: Some(3),
    }];
    let (buf, _) = frame(&app);
    assert!(text(&buf).contains("visible.md"), "{}", text(&buf));
    let home = app.home().unwrap();
    let y = carrel::home::list_geometry(24, 5, app.hints, 1).0;
    assert_eq!(home.row_at(y, 24, 5, app.hints), Some(0));
    assert_eq!(home.resume_row_at(y, 24, 5), None);
    app.on_resize(80, 24);
    let (buf, _) = frame(&app);
    assert!(text(&buf).contains("recent.md"));
}

#[test]
fn compact_paint_and_pointer_agree_across_resize_breakpoints() {
    use carrel::action::{Direction, SearchKey, Span};
    let source = "# 日本語 heading\n\nalpha beta 👩‍💻 gamma delta epsilon needle.\n\n> a quote with words\n\n- nested list words\n\n```rust\n    let long_identifier = 12345;\n```\n\n| heading | value |\n|---|---|\n| alpha | needle |\n\nlast paragraph\n";
    let mut app = App::new("unicode.md".into(), Document::parse(source), 80, 24);
    update(&mut app, Action::SearchOpen(Direction::Forward));
    for c in "needle".chars() {
        update(&mut app, Action::SearchKey(SearchKey::Char(c)));
    }
    update(&mut app, Action::SearchKey(SearchKey::Accept));
    update(&mut app, Action::Scroll(Span::Line, 1));
    let anchor = app.view.anchor;
    let ranges = app.matches.as_ref().unwrap().ranges.clone();
    for (cols, rows) in [
        (40, 6),
        (20, 5),
        (19, 8),
        (12, 3),
        (59, 11),
        (60, 12),
        (80, 24),
    ] {
        app.on_resize(cols, rows);
        assert_eq!(app.view.anchor, anchor);
        assert_eq!(app.matches.as_ref().unwrap().ranges, ranges);
        let (buf, _) = frame(&app);
        let mut checked = 0;
        for y in app.text_y()..app.text_y() + app.text_h() {
            for x in 0..cols {
                if let Some((start, end)) = app.doc_span_at(x, y) {
                    let glyph = buf[(x, y)].symbol();
                    if glyph.trim().is_empty()
                        || app.doc.text[start as usize..end as usize].trim().is_empty()
                    {
                        continue;
                    }
                    assert_eq!(
                        glyph,
                        &app.doc.text[start as usize..end as usize],
                        "{cols}x{rows} cell {x},{y}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }
}

#[test]
fn every_compact_close_button_actually_closes_its_pane() {
    for (open, close) in [
        (Action::HelpToggle, Action::Dismiss),
        (Action::SettingsToggle, Action::SettingsToggle),
        (Action::OutlineToggle, Action::OutlineToggle),
        (Action::MarkListToggle, Action::MarkListToggle),
        (Action::ForwardToggle, Action::ForwardToggle),
        (Action::InfoToggle, Action::InfoToggle),
    ] {
        for (cols, rows) in [(24, 6), (10, 3), (1, 1)] {
            let mut app = app(cols, rows);
            app.marks.push(0);
            update(&mut app, open);
            let (_, painted) = frame(&app);
            assert!(
                has_action(&painted, cols, rows, close),
                "{open:?} at {cols}x{rows}"
            );
            update(&mut app, close);
            assert!(
                app.help.is_none()
                    && app.settings.is_none()
                    && app.outline.is_none()
                    && app.mark_list.is_none()
                    && app.forward.is_none()
                    && !app.info
            );
        }
    }
}

#[test]
fn tiny_chrome_stays_inside_the_window_with_hints_disabled() {
    for cols in 2..20 {
        for rows in 3..12 {
            let mut reader = app(cols, rows);
            reader.hints = false;
            reader.on_resize(cols, rows);
            let _ = frame(&reader);
            let mut home = App::new_home("/docs".into(), vec![], cols, rows);
            home.hints = false;
            let _ = frame(&home);
        }
    }
}

#[test]
fn compact_settings_show_the_whole_label_and_value() {
    for (cols, rows) in [(24, 6), (12, 6), (32, 8)] {
        let mut app = app(cols, rows);
        app.max_width = 0;
        update(&mut app, Action::SettingsToggle);
        for selected in 0..carrel::app::settings_rows(&app).len() {
            app.settings = Some(selected);
            let setting = carrel::app::settings_rows(&app).remove(selected);
            let (buf, painted) = frame(&app);
            let target = painted
                .targets
                .as_slice()
                .iter()
                .find(|t| t.action == Action::SettingsPickAt(selected as u32))
                .unwrap();
            let z = target.zone;
            let shown: String = (z.y..z.y + z.h)
                .flat_map(|y| (z.x..z.x + z.w).map(move |x| (x, y)))
                .map(|at| buf[at].symbol())
                .collect();
            let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            assert!(
                compact(&shown).contains(&compact(setting.label)),
                "{cols}x{rows} missing {} in {shown:?}",
                setting.label
            );
            assert!(
                compact(&shown).contains(&compact(&setting.value)),
                "{cols}x{rows} missing {} in {shown:?}",
                setting.value
            );
        }
    }
}

#[test]
fn long_search_keeps_its_tail_and_feedback_visible() {
    use carrel::action::{Direction, SearchKey};
    let mut app = app(24, 6);
    update(&mut app, Action::SearchOpen(Direction::Forward));
    for c in "a-long-query-with-日本語-END".chars() {
        update(&mut app, Action::SearchKey(SearchKey::Char(c)));
    }
    let (buf, _) = frame(&app);
    let status: String = (0..24).map(|x| buf[(x, 5)].symbol()).collect();
    assert!(
        status.contains("END") && status.contains("no matches"),
        "{status}"
    );
    let mut app = app_for_search_count();
    update(&mut app, Action::SearchOpen(Direction::Forward));
    update(&mut app, Action::SearchKey(SearchKey::Char('a')));
    update(&mut app, Action::SearchKey(SearchKey::Accept));
    let (buf, _) = frame(&app);
    let status: String = (0..24).map(|x| buf[(x, 5)].symbol()).collect();
    assert!(status.contains("1/2"), "{status}");
}
fn app_for_search_count() -> App {
    App::new(
        "a-very-long-filename-hiding-the-count.md".into(),
        Document::parse("a a"),
        24,
        6,
    )
}

#[test]
fn shrinking_a_pane_blocks_hidden_choices_and_note_edits() {
    use carrel::action::NoteKey;
    let mut app = app(80, 24);
    update(&mut app, Action::SettingsToggle);
    app.settings = Some(2);
    let hints = app.hints;
    app.on_resize(8, 2);
    update(&mut app, Action::SettingsAdjust(1));
    assert_eq!(app.hints, hints);
    update(&mut app, Action::SettingsToggle);
    assert!(app.settings.is_none());
    app.on_resize(80, 24);
    update(
        &mut app,
        Action::MenuOpen {
            at: (0, 0),
            byte: None,
        },
    );
    let index = app
        .menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .position(|i| i.action == Some(Action::ThemeCycle))
        .unwrap();
    app.menu.as_mut().unwrap().selected = Some(index);
    app.on_resize(8, 2);
    update(&mut app, Action::MenuChoose);
    assert!(!app.theme_cycle && app.menu.is_some());
    update(&mut app, Action::MenuClose);
    app.on_resize(80, 24);
    update(&mut app, Action::NoteEdit);
    update(&mut app, Action::NoteInput(NoteKey::Char('a')));
    let before = app.notes.draft.as_ref().unwrap().text.clone();
    app.on_resize(8, 2);
    update(&mut app, Action::NoteInput(NoteKey::Char('b')));
    carrel::annotation_state::paste(&mut app, "hidden paste");
    update(&mut app, Action::NoteInput(NoteKey::Save));
    assert_eq!(app.notes.draft.as_ref().unwrap().text, before);
    assert!(app.notes.entries.is_empty());
}

#[test]
fn selected_outline_and_link_show_the_distinguishing_suffix() {
    let heading = "shared heading prefix repeated repeated unique-ending";
    let mut app = App::new(
        "t.md".into(),
        Document::parse(&format!(
            "###### {heading}\n\n[link](https://example.com/shared/shared/unique-destination)"
        )),
        24,
        8,
    );
    update(&mut app, Action::OutlineToggle);
    let (buf, _) = frame(&app);
    let joined = text(&buf).split_whitespace().collect::<String>();
    assert!(joined.contains("unique-ending"), "{}", text(&buf));
    update(&mut app, Action::OutlineToggle);
    update(&mut app, Action::ForwardToggle);
    let (buf, _) = frame(&app);
    let joined = text(&buf).split_whitespace().collect::<String>();
    assert!(joined.contains("unique-destination"), "{}", text(&buf));
}

#[test]
fn home_search_and_filter_keep_the_query_tail_and_count() {
    use carrel::home::HomeMode;
    let mut app = App::new_home("/empty".into(), Vec::new(), 24, 6);
    for mode in [HomeMode::Filter, HomeMode::Search] {
        let home = app.home_mut().unwrap();
        home.mode = mode;
        home.filter = "shared-prefix-long-query-END".into();
        home.query.clone_from(&home.filter);
        home.grep_done = true;
        let (buf, _) = frame(&app);
        let status: String = (0..24).map(|x| buf[(x, 5)].symbol()).collect();
        assert!(status.contains("END") && status.contains('0'), "{status}");
    }
}

#[test]
fn tail_elision_preserves_graphemes_and_stays_within_its_columns() {
    use carrel::layout::tail_text;
    let text = "a-long-prefix-日本語-e\u{301}👩‍💻";
    for width in 0..40 {
        let shown = tail_text(text, width);
        assert!(
            carrel_core::display_width(&shown) <= width,
            "{width}: {shown}"
        );
        if let Some(tail) = shown.strip_prefix('…') {
            assert!(text.ends_with(tail));
            assert!(!tail.starts_with('\u{301}') && !tail.starts_with('\u{200d}'));
        }
    }
    assert_eq!(tail_text("prefix-e\u{301}", 2), "…e\u{301}");
    assert_eq!(tail_text("prefix-👩‍💻", 3), "…👩‍💻");
}
