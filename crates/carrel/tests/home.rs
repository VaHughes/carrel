//! Home screen to reader, across the seam that matters to a user.

use carrel::action::{Action, SearchKey, Span};
use carrel::app::{App, Outcome, update};
use carrel::scan;

#[test]
fn filter_then_open_then_read() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("alpha.md"), "# Alpha\n\nfirst document").unwrap();
    std::fs::write(d.path().join("beta.md"), "# Beta\n\nsecond document").unwrap();

    let (cached, _) = scan::walk_blocking(d.path());
    assert_eq!(cached.len(), 2);

    let mut app = App::new_home(d.path().into(), cached, 60, 16);

    // Type "beta" — one match survives.
    for c in "beta".chars() {
        update(&mut app, Action::HomeKey(SearchKey::Char(c)));
    }
    assert_eq!(app.home().unwrap().filtered.len(), 1);
    assert_eq!(update(&mut app, Action::HomeOpen), Outcome::Redraw);

    // In the reader, with the right document, and it scrolls.
    assert!(!app.is_home());
    assert!(app.doc.text.contains("second document"), "{}", app.doc.text);
    assert_eq!(app.path, "beta.md");
    assert_eq!(
        update(&mut app, Action::Scroll(Span::Line, 1)),
        Outcome::Redraw
    );
}

#[test]
fn a_scan_that_finds_nothing_leaves_a_usable_home_screen() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::new_home(d.path().into(), vec![], 60, 16);
    assert_eq!(update(&mut app, Action::HomeOpen), Outcome::Idle);
    assert!(app.is_home());
    assert_eq!(update(&mut app, Action::Quit), Outcome::Quit);
}

/// The cache is painted first and the walk rediscovers the same files. Without
/// dedup every entry would appear twice; without the drop, deleted files linger.
#[test]
fn the_live_walk_reconciles_with_the_cache() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("real.md"), "# real").unwrap();

    // A cache that is one file stale and one file short.
    let stale = vec![scan::Entry {
        path: d.path().join("deleted.md"),
        mtime: std::time::SystemTime::UNIX_EPOCH,
    }];
    let mut app = App::new_home(d.path().into(), stale, 60, 16);
    assert_eq!(app.home().unwrap().entries.len(), 1, "cache painted first");

    for msg in scan::spawn(d.path()) {
        match msg {
            scan::Msg::Found(e) => app.home_mut().unwrap().push(e),
            scan::Msg::Done { unreadable } => app.home_mut().unwrap().finish_scan(unreadable),
        };
    }

    let h = app.home().unwrap();
    assert!(!h.scanning);
    let names: Vec<_> = h
        .entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        ["real.md"],
        "stale entry dropped, real one kept once"
    );
}

mod picker {
    use carrel::action::{Action, SearchKey};
    use carrel::app::{App, Outcome, update};
    use carrel::home::HomeMode;
    use carrel_core::Document;

    /// The picker as `d` leaves it: prefilled with the launch directory.
    ///
    /// `launch_dir` is set explicitly, exactly as the binary sets it — a test
    /// that left it `None` would fall through to the home root and so never
    /// exercise the path a reader actually walks.
    fn picker_app(d: &tempfile::TempDir) -> App {
        let mut app = App::new_home(d.path().into(), vec![], 60, 16);
        app.launch_dir = Some(d.path().into());
        update(&mut app, Action::HomeKey(SearchKey::Cancel)); // -> Normal
        update(&mut app, Action::PickerOpen);
        assert_eq!(app.home().unwrap().mode, HomeMode::Picker);
        app
    }

    /// The picker with the prefill cleared — one Esc, which is what someone
    /// heading somewhere unrelated presses before typing an absolute path.
    fn picker_app_cleared(d: &tempfile::TempDir) -> App {
        let mut app = picker_app(d);
        update(&mut app, Action::HomeKey(SearchKey::Cancel));
        assert!(app.home().unwrap().picker.typed.is_empty());
        assert_eq!(app.home().unwrap().mode, HomeMode::Picker);
        app
    }

    /// The maintainer's report, 2026-08-29: `d` opened on an empty input, so
    /// `/live` meant the filesystem root and reaching a sibling of the
    /// directory already on screen meant typing the whole path from `/`.
    #[test]
    fn the_picker_opens_on_the_directory_carrel_was_run_from() {
        let d = tempfile::tempdir().unwrap();
        for sub in ["live", "archive"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        let app = picker_app(&d);
        let h = app.home().unwrap();
        assert_eq!(
            h.picker.typed,
            format!("{}/", d.path().display()),
            "the input opens on the launch directory, slash and all",
        );
        assert_eq!(
            h.picker.roots,
            vec![
                d.path().to_path_buf(),
                d.path().join("archive"),
                d.path().join("live"),
            ],
            "the launch directory leads what you can reach from it",
        );
    }

    /// The maintainer's report, 2026-09-01: with a saved `root =` in the
    /// config, `d` opened on the last directory read in rather than the one
    /// the command had just been typed in — and Enter went there.
    #[test]
    fn the_launch_directory_beats_the_saved_root_the_screen_opens_on() {
        let launched_in = tempfile::tempdir().unwrap();
        std::fs::create_dir(launched_in.path().join("notes")).unwrap();
        let saved_root = tempfile::tempdir().unwrap();

        // What startup does with `root = …` on file: the screen opens on the
        // saved root, the shell is somewhere else entirely.
        let mut app = App::new_home(saved_root.path().into(), vec![], 60, 16);
        app.launch_dir = Some(launched_in.path().into());
        update(&mut app, Action::HomeKey(SearchKey::Cancel)); // -> Normal
        update(&mut app, Action::PickerOpen);

        let h = app.home().unwrap();
        assert_eq!(
            h.picker.typed,
            format!("{}/", launched_in.path().display()),
            "the input opens where the command was typed",
        );
        assert_eq!(
            h.picker.roots.first(),
            Some(&launched_in.path().to_path_buf()),
            "and the highlight is already on it: {:?}",
            h.picker.roots,
        );

        // So Enter alone — no typing — reads where you are.
        update(&mut app, Action::PickerChoose);
        assert_eq!(app.home().unwrap().root, launched_in.path());
    }

    /// The fallback for a reader whose working directory has been deleted out
    /// from under them: no prefill, so the default menu still has something.
    #[test]
    fn a_launch_directory_that_is_gone_falls_back_to_the_default_menu() {
        let d = tempfile::tempdir().unwrap();
        let mut app = App::new_home(d.path().into(), vec![], 60, 16);
        app.launch_dir = Some(std::path::PathBuf::from("/no/such/place/at/all"));
        if let Some(h) = app.home_mut() {
            h.places = vec![std::path::PathBuf::from("/fav/notes")];
        }
        update(&mut app, Action::HomeKey(SearchKey::Cancel)); // -> Normal
        update(&mut app, Action::PickerOpen);

        let h = app.home().unwrap();
        assert!(h.picker.typed.is_empty(), "nothing to continue from");
        assert_eq!(
            h.picker.roots.first(),
            Some(&std::path::PathBuf::from("/fav/notes")),
            "so the remembered places lead: {:?}",
            h.picker.roots,
        );
    }

    /// Both typing styles have to continue from here — the leading slash is a
    /// separator, not a jump to `/`.
    #[test]
    fn typing_continues_from_the_current_directory_with_or_without_a_slash() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("live")).unwrap();
        std::fs::write(d.path().join("live").join("x.md"), "# x").unwrap();

        for typed in ["/live", "live"] {
            let mut app = picker_app(&d);
            for c in typed.chars() {
                update(&mut app, Action::HomeKey(SearchKey::Char(c)));
            }
            assert_eq!(
                app.home().unwrap().picker.roots,
                vec![d.path().join("live")],
                "{typed:?} must resolve under the current directory",
            );
            assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
            assert_eq!(app.home().unwrap().root, d.path().join("live"));
        }
    }

    /// The prefill's escape hatch. Somewhere unrelated is still one Esc away,
    /// and this is the only way to reach it — the input is text, so an
    /// absolute path typed after the prefill would hang off the end of it.
    #[test]
    fn escape_clears_the_prefill_so_an_unrelated_path_can_still_be_typed() {
        let d = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        assert!(!app.home().unwrap().picker.typed.is_empty());

        update(&mut app, Action::HomeKey(SearchKey::Cancel));
        let h = app.home().unwrap();
        assert!(h.picker.typed.is_empty(), "first Esc clears the prefill");
        assert_eq!(h.mode, HomeMode::Picker, "and the picker stays up");

        for c in target.path().to_str().unwrap().chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::PickerChoose);
        assert_eq!(app.home().unwrap().root, target.path());
    }

    #[test]
    fn the_home_screen_opens_in_normal_mode_with_the_menu() {
        let d = tempfile::tempdir().unwrap();
        let app = App::new_home(d.path().into(), vec![], 60, 16);
        assert_eq!(
            app.home().unwrap().mode,
            HomeMode::Normal,
            "the menu, not the filter, greets you"
        );
    }

    #[test]
    fn cancelling_the_picker_returns_to_the_menu_it_was_opened_from() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        update(&mut app, Action::PickerCancel);
        assert_eq!(app.home().unwrap().mode, HomeMode::Normal);
    }

    #[test]
    fn typing_a_path_completes_it_and_choosing_it_changes_the_root() {
        let d = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("x.md"), "# x").unwrap();

        let mut app = picker_app_cleared(&d);
        for c in target.path().to_str().unwrap().chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        let h = app.home().unwrap();
        assert_eq!(
            h.picker.typed.as_str(),
            target.path().to_str().unwrap(),
            "typing must fill the picker's path",
        );
        assert_eq!(
            h.picker.roots,
            vec![target.path().to_path_buf()],
            "and the fully typed path is the one match",
        );
        assert!(
            h.filter.is_empty(),
            "and must NOT leak into the hidden filter"
        );

        assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
        assert_eq!(app.home().unwrap().root, target.path());
        // This test once wrote its tempdir into the developer's REAL config
        // on every `cargo test` run — App::config_dir is None here, so the
        // choice must persist nowhere.
        assert_eq!(app.config_dir, None);
    }

    /// The maintainer's report, 2026-08-21: choosing a directory dropped
    /// straight into the filter, so the next keystroke silently hid files.
    #[test]
    fn choosing_a_directory_lands_in_the_menu_not_the_filter() {
        let d = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("x.md"), "# x").unwrap();

        let mut app = picker_app_cleared(&d);
        for c in target.path().to_str().unwrap().chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::PickerChoose);
        let h = app.home().unwrap();
        assert_eq!(h.root, target.path());
        assert_eq!(h.mode, HomeMode::Normal);
    }

    #[test]
    fn a_partial_path_offers_every_directory_that_matches_it() {
        let d = tempfile::tempdir().unwrap();
        for sub in ["alpha", "album", "beta"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        // Two letters, because the picker already holds the directory they
        // are a prefix of.
        let mut app = picker_app(&d);
        for c in "al".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        assert_eq!(
            app.home().unwrap().picker.roots,
            vec![d.path().join("album"), d.path().join("alpha")],
        );
        // Backspacing widens the list again — the completion is live, and it
        // bottoms out at the prefill rather than at nothing.
        update(&mut app, Action::HomeKey(SearchKey::Backspace));
        update(&mut app, Action::HomeKey(SearchKey::Backspace));
        assert_eq!(
            app.home().unwrap().picker.roots,
            vec![
                d.path().join("album"),
                d.path().join("alpha"),
                d.path().join("beta"),
            ],
            "a trailing slash lists the directory whole",
        );
    }

    #[test]
    fn escape_clears_the_typed_path_before_it_closes_the_picker() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        update(&mut app, Action::HomeKey(SearchKey::Char('/')));
        update(&mut app, Action::HomeKey(SearchKey::Cancel));
        let h = app.home().unwrap();
        assert!(h.picker.typed.is_empty(), "first Esc clears the path");
        assert_eq!(h.mode, HomeMode::Picker, "and the picker stays up");
        update(&mut app, Action::HomeKey(SearchKey::Cancel));
        assert_eq!(app.home().unwrap().mode, HomeMode::Normal, "second closes");
    }

    #[test]
    fn enter_follows_the_highlight_not_the_typed_prefix() {
        // The trap this guards against: type a prefix, move the highlight
        // down to the second match, press Enter. The typed text must not
        // hijack the choice — it used to, which left the picker unable to
        // choose anything but the abandoned path until Esc.
        let d = tempfile::tempdir().unwrap();
        for sub in ["alpha", "album"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        let mut app = picker_app(&d);
        for c in "al".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::HomeMove(1)); // down to the second match
        let expected = app.home().unwrap().picker.roots[1].clone();
        assert_eq!(expected, d.path().join("alpha"));
        assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
        assert_eq!(app.home().unwrap().root, expected);
    }

    #[test]
    fn a_typed_path_that_is_not_a_directory_is_refused_out_loud() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app_cleared(&d);
        let before = app.home().unwrap().root.clone();
        // Nothing matches, so Enter falls through to the typed text — which
        // has to be complained about rather than silently doing nothing.
        for c in "/no/such/place".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        assert!(app.home().unwrap().picker.roots.is_empty());
        assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
        let h = app.home().unwrap();
        assert_eq!(h.root, before, "nothing to choose, nothing chosen");
        assert_eq!(h.mode, HomeMode::Picker, "and the picker stays up");
        assert!(h.note.is_some(), "with a complaint on the status bar");

        // Erasing it all is not a path "" that earns the same complaint —
        // the defaults come back and Enter takes the highlighted one.
        for _ in 0.."/no/such/place".len() {
            update(&mut app, Action::HomeKey(SearchKey::Backspace));
        }
        let expected = app.home().unwrap().picker.roots[0].clone();
        assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
        assert_eq!(app.home().unwrap().root, expected);
    }

    #[test]
    fn choosing_a_root_persists_into_the_injected_config_dir_only() {
        let d = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        app.config_dir = Some(cfg.path().into());

        update(&mut app, Action::PickerChoose); // the selected listed root
        let chosen = app.home().unwrap().root.clone();
        assert_eq!(
            carrel::config::load_root_in(cfg.path()),
            Some(chosen),
            "the choice must land in the injected directory"
        );
    }

    #[test]
    fn picker_keys_never_leak_into_the_filter_behind_the_overlay() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app_cleared(&d);
        update(&mut app, Action::HomeKey(SearchKey::Char('z')));
        let h = app.home().unwrap();
        assert!(
            h.filter.is_empty(),
            "picker keys must never edit the filter"
        );
        assert_eq!(h.picker.typed, "z");
    }

    #[test]
    fn opening_a_file_clears_pending_key_prefixes() {
        use carrel::keys::Keys;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut keys = Keys::new();
        // A lone 'g' in home Normal mode arms the gg prefix…
        assert_eq!(
            keys.map_home(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                HomeMode::Normal,
            ),
            None,
        );
        keys.reset();
        // …but after reset, the reader's first 'j' must scroll, not vanish.
        let mut app = App::new(
            "t.md".into(),
            Document::parse("a\nb\nc\nd\ne\nf\ng\nh"),
            20,
            4,
        );
        let action = keys
            .map(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), false)
            .expect("j must not be swallowed by a stale prefix");
        assert_eq!(update(&mut app, action), Outcome::Redraw);
    }
}

mod links {
    use carrel::action::Action;
    use carrel::app::{App, Outcome, update};
    use carrel_core::{Document, LinkId};

    /// Two documents in a directory, a.md linking to b.md and to the web.
    fn fixture() -> (tempfile::TempDir, App) {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("a.md"),
            "# A\n\nsee [the notes](b.md) or [the site](https://example.com/x)\n\nfiller\n",
        )
        .unwrap();
        std::fs::write(d.path().join("b.md"), "# B\n\narrived").unwrap();
        let src = std::fs::read_to_string(d.path().join("a.md")).unwrap();
        let mut app = App::new("a.md".into(), Document::parse(&src), 40, 10);
        app.file = Some(d.path().join("a.md"));
        (d, app)
    }

    #[test]
    fn tab_cycles_links_in_document_order_and_wraps() {
        let (_d, mut app) = fixture();
        update(&mut app, Action::LinkStep(1));
        assert_eq!(app.selected_link, Some(LinkId(0)));
        update(&mut app, Action::LinkStep(1));
        assert_eq!(app.selected_link, Some(LinkId(1)));
        update(&mut app, Action::LinkStep(1));
        assert_eq!(app.selected_link, Some(LinkId(0)), "wraps");
        update(&mut app, Action::LinkStep(-1));
        assert_eq!(app.selected_link, Some(LinkId(1)), "backwards wraps too");
    }

    #[test]
    fn following_a_relative_link_opens_it_and_back_returns_to_the_anchor() {
        let (_d, mut app) = fixture();
        update(&mut app, Action::LinkStep(1)); // select b.md
        let anchor_before = app.view.anchor;

        assert_eq!(update(&mut app, Action::LinkFollow), Outcome::Redraw);
        assert!(app.doc.text.contains("arrived"), "must be reading b.md");
        assert_eq!(app.history.len(), 1);

        assert_eq!(update(&mut app, Action::Back), Outcome::Redraw);
        assert!(app.doc.text.contains("filler"), "must be back in a.md");
        assert_eq!(app.view.anchor, anchor_before, "reading position restored");
        assert!(app.history.is_empty());
    }

    /// Was `an_external_link_is_never_followed_only_noted`. It is copied now
    /// rather than only named, but what this guards is unchanged: the READER
    /// does not move, nothing is navigated to, and no program is launched.
    #[test]
    fn an_external_link_is_copied_and_leaves_the_reader_where_it_is() {
        let (_d, mut app) = fixture();
        update(&mut app, Action::LinkStep(1));
        update(&mut app, Action::LinkStep(1)); // select https://example.com/x
        update(&mut app, Action::LinkFollow);
        assert!(app.doc.text.contains("filler"), "still reading a.md");
        assert_eq!(
            app.clipboard.take().as_deref(),
            Some("https://example.com/x"),
            "the URL leaves through the clipboard outbox"
        );
        assert_eq!(app.note.as_deref(), Some("copied to clipboard"));
        assert!(app.history.is_empty(), "nothing to go back to");
    }

    #[test]
    fn a_missing_target_leaves_a_note_and_stays_put() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.md"), "[gone](missing.md)").unwrap();
        let src = std::fs::read_to_string(d.path().join("a.md")).unwrap();
        let mut app = App::new("a.md".into(), Document::parse(&src), 40, 10);
        app.file = Some(d.path().join("a.md"));

        update(&mut app, Action::LinkStep(1));
        update(&mut app, Action::LinkFollow);
        assert!(app.doc.text.contains("gone"), "still on a.md");
        assert!(app.note.as_deref().unwrap_or("").contains("missing.md"));
        assert!(
            app.history.is_empty(),
            "a failed follow must not pollute history"
        );
    }

    #[test]
    fn back_with_empty_history_is_a_no_op() {
        let (_d, mut app) = fixture();
        assert_eq!(update(&mut app, Action::Back), Outcome::Idle);
    }

    #[test]
    fn escape_clears_the_link_selection() {
        let (_d, mut app) = fixture();
        update(&mut app, Action::LinkStep(1));
        assert!(app.selected_link.is_some());
        update(&mut app, Action::Dismiss);
        assert!(app.selected_link.is_none());
    }
}

mod close_file {
    use carrel::action::{Action, SearchKey};
    use carrel::app::{App, Outcome, update};
    use carrel_core::Document;

    #[test]
    fn q_returns_to_the_home_screen_with_state_intact() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("alpha.md"), "# A\n\nbody").unwrap();
        std::fs::write(d.path().join("beta.md"), "# B").unwrap();
        let (cached, _) = carrel::scan::walk_blocking(d.path());
        let mut app = App::new_home(d.path().into(), cached, 60, 16);

        // Filter, open — then close.
        for c in "alpha".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::HomeOpen);
        assert!(!app.is_home());
        assert!(
            app.home_stash.is_some(),
            "the home screen is stashed, not dropped"
        );

        assert_eq!(update(&mut app, Action::CloseFile), Outcome::Redraw);
        assert!(app.is_home(), "q goes back to the library");
        let h = app.home().unwrap();
        assert_eq!(h.filter, "alpha", "the filter survived the round trip");
        assert_eq!(h.entries.len(), 2, "entries survived too");
    }

    #[test]
    fn q_quits_when_the_file_was_opened_directly() {
        let mut app = App::new("t.md".into(), Document::parse("# T"), 40, 10);
        assert_eq!(update(&mut app, Action::CloseFile), Outcome::Quit);
    }
}

/// The guard that makes click-to-open trustworthy: whatever file name is
/// painted on a row, clicking that row must resolve to that same file.
///
/// A frame test alone cannot see a one-row offset, and neither can a unit
/// test of the geometry — only the round trip can. Verified to fail on a
/// deliberate off-by-one before being trusted.
#[test]
fn clicking_a_row_resolves_to_the_file_painted_on_it() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let d = tempfile::tempdir().unwrap();
    for n in ["alpha.md", "beta.md", "gamma.md", "delta.md"] {
        std::fs::write(d.path().join(n), "# x").unwrap();
    }
    let (cached, _) = scan::walk_blocking(d.path());

    // Both a banner-sized terminal and one too small for it, because the
    // list's top row differs between them.
    for (cols, rows) in [(100u16, 40u16), (40, 12)] {
        let app = App::new_home(d.path().into(), cached.clone(), cols, rows);
        let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        t.draw(|f| carrel::render::draw(f, &app)).unwrap();
        let buf = t.backend().buffer().clone();

        let home = app.home().unwrap();
        let mut checked = 0;
        for row in 0..rows {
            let Some(i) = home.row_at(row, cols, rows, app.hints) else {
                continue;
            };
            let painted: String = (0..cols).map(|c| buf[(c, row)].symbol()).collect();
            let expected = home.entries[home.filtered[i]]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            assert!(
                painted.contains(&expected),
                "{cols}x{rows} row {row}: click resolves to {expected:?} \
                 but the row paints {:?}",
                painted.trim()
            );
            checked += 1;
        }
        assert_eq!(
            checked, 4,
            "{cols}x{rows}: every file row should be hittable"
        );
    }
}

/// Same round trip for the directory picker overlay: whatever path is painted
/// on a row, clicking that row must resolve to that entry.
#[test]
fn clicking_a_picker_row_resolves_to_the_directory_painted_on_it() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.md"), "# a").unwrap();
    let (cached, _) = scan::walk_blocking(d.path());
    let (cols, rows) = (80u16, 24u16);
    let mut app = App::new_home(d.path().into(), cached, cols, rows);
    update(&mut app, Action::PickerOpen);
    // A root deeper than the overlay is wide, on purpose: the picker probes
    // the real machine for candidates, and on a COPR builder the working
    // directory alone is this deep — the row paints a clipped path and must
    // still resolve.
    app.home_mut()
        .unwrap()
        .picker
        .roots
        .push(std::path::PathBuf::from(
            "/builddir/build/BUILD/carrel-2026.8.17-build/carrel-2026.8.17/crates/carrel",
        ));

    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| carrel::render::draw(f, &app)).unwrap();
    let buf = t.backend().buffer().clone();

    let home = app.home().unwrap();
    // The painter writes "▸ {path}" one cell inside the box and clips it to
    // the box with `set_stringn`, so a row shows at most `w - 3` cells of the
    // path itself (ASCII here, so cells == chars). The contract under test is
    // that a row paints exactly as much of its entry as fits — not that every
    // entry fits.
    let entries = u16::try_from(home.picker_entries()).unwrap();
    let (_, _, box_w, _) = carrel::home::picker_geometry(cols, rows, entries);
    let budget = usize::from(box_w) - 3;
    let mut checked = 0;
    for row in 0..rows {
        let Some(i) = home.picker_row_at(cols / 2, row, cols, rows) else {
            continue;
        };
        let painted: String = (0..cols).map(|c| buf[(c, row)].symbol()).collect();
        let expected = home.picker.roots[i].display().to_string();
        let shown: String = expected.chars().take(budget).collect();
        assert!(
            painted.contains(&shown),
            "row {row}: click resolves to {expected:?} but the row paints {:?}",
            painted.trim()
        );
        checked += 1;
    }
    let (_, _, visible) = home.picker_view(cols, rows);
    assert_eq!(
        checked,
        home.picker_entries().min(visible),
        "every VISIBLE picker row should be hittable"
    );
    assert!(checked > 0, "the picker painted nothing to click");
}

#[test]
fn a_filter_ranks_fuzzily_instead_of_substring_order() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("unread-note.md"), "# U").unwrap();
    std::fs::write(d.path().join("readme.md"), "# R").unwrap();

    let (cached, _) = scan::walk_blocking(d.path());
    let mut app = App::new_home(d.path().into(), cached.clone(), 60, 16);

    // `read` lives inside both paths; the one that starts the word ranks first.
    for c in "read".chars() {
        update(&mut app, Action::HomeKey(SearchKey::Char(c)));
    }
    let h = app.home().unwrap();
    assert_eq!(h.filtered.len(), 2);
    let first = h.filtered[0];
    assert!(
        h.entries[first].path.ends_with("readme.md"),
        "{:?}",
        h.entries[first].path
    );

    // And a subsequence that is not a substring still finds its file.
    let mut app2 = App::new_home(d.path().into(), cached, 60, 16);
    for c in "udn".chars() {
        update(&mut app2, Action::HomeKey(SearchKey::Char(c)));
    }
    let h2 = app2.home().unwrap();
    assert_eq!(h2.filtered.len(), 1);
    assert!(h2.entries[h2.filtered[0]].path.ends_with("unread-note.md"));
}

/// Places lead the EMPTY menu — the picker opens on the launch directory, and
/// Esc clears that prefill to reach the remembered places. Leading the opened
/// list was what put the last directory read in ahead of the one the command
/// was typed in; see `the_launch_directory_beats_the_saved_root…`.
#[test]
fn places_lead_the_empty_picker_menu_and_a_choice_becomes_one() {
    let d = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    std::fs::write(cfg.path().join("config"), "place = /fav/notes\n").unwrap();

    let mut app = App::new_home(d.path().into(), vec![], 60, 16);
    app.config_dir = Some(cfg.path().into());
    app.launch_dir = Some(d.path().into());
    // Startup prefs load places; drive the same path the binary uses.
    if let Some(h) = app.home_mut() {
        h.places = carrel::config::load_places_in(cfg.path());
    }
    update(&mut app, Action::HomeKey(SearchKey::Cancel)); // -> Normal
    update(&mut app, Action::PickerOpen);
    assert_eq!(
        app.home().unwrap().picker.roots.first(),
        Some(&d.path().to_path_buf()),
        "the launch directory leads the opened list, not a place",
    );
    update(&mut app, Action::HomeKey(SearchKey::Cancel)); // clear the prefill
    let h = app.home().unwrap();
    assert_eq!(
        h.picker.roots.first(),
        Some(&std::path::PathBuf::from("/fav/notes"))
    );

    // Choosing a directory records it as a place, newest first.
    update(&mut app, Action::PickerCancel);
    let target = d.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    update(&mut app, Action::PickerOpen);
    if let Some(h) = app.home_mut() {
        h.picker.typed = format!("{}", target.display());
        h.picker.roots = vec![target.clone()];
        h.picker.selected = 0;
    }
    update(&mut app, Action::PickerChoose);
    let places = carrel::config::load_places_in(cfg.path());
    assert_eq!(places.first(), Some(&target), "{places:?}");
    assert!(places.contains(&std::path::PathBuf::from("/fav/notes")));
}
