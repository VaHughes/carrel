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

    /// The picker as `d` leaves it: browsing the launch directory, nothing
    /// typed, browsable before a single keystroke.
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

    /// The dialog opens ON somewhere: parent, itself, children, places — not
    /// on an empty prompt waiting for a path to be typed into it.
    #[test]
    fn the_picker_opens_browsing_where_carrel_was_run_from() {
        let d = tempfile::tempdir().unwrap();
        for sub in ["live", "archive"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        let app = picker_app(&d);
        let h = app.home().unwrap();
        assert!(h.picker.typed.is_empty(), "nothing to type over");
        assert_eq!(h.picker.browsing, d.path());
        assert_eq!(
            &h.picker.roots[..4],
            &[
                d.path().parent().unwrap().to_path_buf(),
                d.path().to_path_buf(),
                d.path().join("archive"),
                d.path().join("live"),
            ],
            "parent, here, children: {:?}",
            h.picker.roots,
        );
        assert_eq!(
            h.picker.roots[h.picker.selected],
            d.path().to_path_buf(),
            "and the highlight is already on here",
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
            h.picker.browsing,
            launched_in.path(),
            "the dialog browses where the command was typed",
        );
        assert!(
            h.picker.roots.contains(&launched_in.path().to_path_buf()),
            "and here is on the list: {:?}",
            h.picker.roots,
        );

        // The highlight parks on "here", so Enter alone — no typing —
        // reads where you are.
        assert_eq!(
            h.picker.roots[h.picker.selected],
            launched_in.path().to_path_buf(),
        );
        update(&mut app, Action::PickerChoose);
        assert_eq!(app.home().unwrap().root, launched_in.path());
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

    /// A bare word filters the neighbourhood fuzzily; erasing it brings the
    /// neighbourhood back.
    #[test]
    fn typing_a_word_filters_and_erasing_it_restores_the_neighbourhood() {
        let d = tempfile::tempdir().unwrap();
        for sub in ["zephyrine", "zephyrion", "plain"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        let mut app = picker_app(&d);
        for c in "zeph".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        let mut roots = app.home().unwrap().picker.roots.clone();
        roots.sort();
        assert_eq!(
            roots,
            vec![d.path().join("zephyrine"), d.path().join("zephyrion")],
            "the filter narrows; it does not complete a prefix",
        );
        for _ in 0.."zeph".len() {
            update(&mut app, Action::HomeKey(SearchKey::Backspace));
        }
        let h = app.home().unwrap();
        assert!(h.picker.typed.is_empty());
        assert!(
            h.picker.roots.contains(&d.path().join("plain")),
            "erased back to the neighbourhood: {:?}",
            h.picker.roots,
        );
    }

    /// A filter with a `/` in it is a path, not text: an absolute path typed
    /// in full still resolves, even somewhere unrelated.
    #[test]
    fn typing_a_path_completes_it_and_choosing_it_changes_the_root() {
        let d = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("x.md"), "# x").unwrap();

        let mut app = picker_app(&d);
        for c in target.path().to_str().unwrap().chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        let h = app.home().unwrap();
        assert_eq!(
            h.picker.typed.as_str(),
            target.path().to_str().unwrap(),
            "typing must fill the picker's filter",
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

        let mut app = picker_app(&d);
        for c in target.path().to_str().unwrap().chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::PickerChoose);
        let h = app.home().unwrap();
        assert_eq!(h.root, target.path());
        assert_eq!(h.mode, HomeMode::Normal);
    }

    #[test]
    fn escape_clears_the_filter_before_it_closes_the_picker() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        update(&mut app, Action::HomeKey(SearchKey::Char('z')));
        update(&mut app, Action::HomeKey(SearchKey::Cancel));
        let h = app.home().unwrap();
        assert!(h.picker.typed.is_empty(), "first Esc clears the filter");
        assert_eq!(h.mode, HomeMode::Picker, "and the picker stays up");
        update(&mut app, Action::HomeKey(SearchKey::Cancel));
        assert_eq!(app.home().unwrap().mode, HomeMode::Normal, "second closes");
    }

    #[test]
    fn enter_follows_the_highlight_not_the_typed_filter() {
        // The trap this guards against: type a filter, move the highlight
        // down to the second match, press Enter. The typed text must not
        // hijack the choice — it used to, which left the picker unable to
        // choose anything but the abandoned path until Esc.
        let d = tempfile::tempdir().unwrap();
        for sub in ["zephyrine", "zephyrion"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        let mut app = picker_app(&d);
        for c in "zeph".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::HomeMove(1)); // down to the second match
        let expected = app.home().unwrap().picker.roots[1].clone();
        assert!(expected.starts_with(d.path()), "{expected:?}");
        assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
        assert_eq!(app.home().unwrap().root, expected);
    }

    #[test]
    fn a_typed_path_that_is_not_a_directory_is_refused_out_loud() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
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
        // the neighbourhood comes back and Enter takes the highlighted one.
        for _ in 0.."/no/such/place".len() {
            update(&mut app, Action::HomeKey(SearchKey::Backspace));
        }
        let h = app.home().unwrap();
        let expected = h.picker.roots[h.picker.selected].clone();
        assert_eq!(update(&mut app, Action::PickerChoose), Outcome::Redraw);
        assert_eq!(app.home().unwrap().root, expected);
    }

    /// Drilling in and climbing browse without choosing: the screen's root
    /// does not move until Enter says so.
    #[test]
    fn descend_and_climb_browse_without_choosing() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("live")).unwrap();
        std::fs::create_dir(d.path().join("live").join("inner")).unwrap();
        let mut app = picker_app(&d);

        let live = app
            .home()
            .unwrap()
            .picker
            .roots
            .iter()
            .position(|r| r == &d.path().join("live"))
            .unwrap();
        update(&mut app, Action::PickerSelect(live));
        update(&mut app, Action::PickerDescend);
        let h = app.home().unwrap();
        assert_eq!(h.picker.browsing, d.path().join("live"));
        assert_eq!(h.root, d.path(), "browsing is not choosing");
        assert_eq!(h.mode, HomeMode::Picker, "the dialog stays up");
        assert!(
            h.picker
                .roots
                .contains(&d.path().join("live").join("inner"))
        );

        update(&mut app, Action::PickerUp);
        let h = app.home().unwrap();
        assert_eq!(h.picker.browsing, d.path());
        assert_eq!(h.root, d.path());
    }

    /// Backspace on an empty filter climbs, the keyboard twin of the `..` row.
    #[test]
    fn backspace_on_an_empty_filter_climbs() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        update(&mut app, Action::HomeKey(SearchKey::Backspace));
        let h = app.home().unwrap();
        assert_eq!(h.picker.browsing, d.path().parent().unwrap());
        assert_eq!(h.mode, HomeMode::Picker);
    }

    #[test]
    fn choosing_a_root_persists_into_the_injected_config_dir_only() {
        let d = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
        app.config_dir = Some(cfg.path().into());

        // The highlight parks on "here", so choosing at once reads it.
        assert_eq!(
            app.home().unwrap().picker.roots[app.home().unwrap().picker.selected],
            d.path().to_path_buf(),
        );
        update(&mut app, Action::PickerChoose);
        let chosen = app.home().unwrap().root.clone();
        assert_eq!(chosen, d.path());
        assert_eq!(
            carrel::config::load_root_in(cfg.path()),
            Some(chosen),
            "the choice must land in the injected directory"
        );
    }

    #[test]
    fn picker_keys_never_leak_into_the_filter_behind_the_overlay() {
        let d = tempfile::tempdir().unwrap();
        let mut app = picker_app(&d);
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
        assert_eq!(app.note.as_deref(), Some("copied the link"));
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

mod search_results {
    use carrel::action::{Action, SearchKey};
    use carrel::app::{App, update};
    use carrel::grep::{Hit, HitLine};

    /// The whole loop through the public API: search, read the hits as a
    /// document, follow a match to its line, come back to the library.
    #[test]
    fn results_read_as_a_document_and_each_match_jumps_to_its_line() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("doc.md");
        std::fs::write(&f, "first\nthe needle sits here\nlast\n").unwrap();
        let (cached, _) = carrel::scan::walk_blocking(d.path());
        let mut app = App::new_home(d.path().into(), cached, 60, 20);
        app.library_root = Some(d.path().to_path_buf());

        update(&mut app, Action::HomeSearchMode);
        for c in "needle".chars() {
            update(&mut app, Action::HomeKey(SearchKey::Char(c)));
        }
        // The event loop normally streams these in; inject directly.
        if let Some(h) = app.home_mut() {
            h.hits.push(Hit {
                path: f.clone(),
                count: 1,
                first_line: "the needle sits here".into(),
                matches: vec![HitLine {
                    lineno: 2,
                    line: "the needle sits here".into(),
                }],
            });
        }
        update(&mut app, Action::HomeOpenResults);
        assert!(!app.is_home(), "Tab reads the results");
        assert!(
            app.doc.text.contains("doc.md — 1 match"),
            "a section per file"
        );

        // The first link is the first match: following it opens the file.
        update(&mut app, Action::LinkOpen(0));
        assert_eq!(app.file.as_deref(), Some(f.as_path()));

        // And `q` from there returns to the search it came from.
        update(&mut app, Action::CloseFile);
        assert!(app.is_home());
        let h = app.home().unwrap();
        assert_eq!(h.query, "needle", "the query survived the round trip");
        assert_eq!(h.hits.len(), 1, "the hits did too");
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

/// Same round trip for the library dialog: whatever a row paints, clicking
/// that row must resolve to the entry behind it.
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
    // The painter writes rows as `▸/␣ mark path` one cell inside the box and
    // clips to it, so a row shows at most `w - 5` cells of the path itself
    // (ASCII here, so cells == chars). Two rows paint labels instead of
    // paths: the parent is `..`, and where you are carries `· here`.
    let entries = u16::try_from(home.picker_entries()).unwrap();
    let (_, _, box_w, _) = carrel::home::picker_geometry(cols, rows, entries);
    let budget = usize::from(box_w) - 5;
    let browsing = home.picker.browsing.clone();
    let parent = browsing.parent().map(std::path::Path::to_path_buf);
    let mut checked = 0;
    for row in 0..rows {
        let Some(i) = home.picker_row_at(cols / 2, row, cols, rows) else {
            continue;
        };
        let painted: String = (0..cols).map(|c| buf[(c, row)].symbol()).collect();
        let root = &home.picker.roots[i];
        if Some(root) == parent.as_ref() {
            assert!(
                painted.contains(".."),
                "row {row}: the parent must paint as `..`, paints {painted:?}",
            );
        } else {
            let path = root.display().to_string();
            let shown: String = path.chars().take(budget).collect();
            assert!(
                painted.contains(&shown),
                "row {row}: click resolves to {path:?} but the row paints {painted:?}",
            );
            if root == &browsing {
                assert!(
                    painted.contains("· here"),
                    "row {row}: where you are must say so: {painted:?}",
                );
            }
        }
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

/// Places are listed in the opened dialog — the dialog opens browsing the
/// launch directory, and the remembered favourites are part of its rows.
/// Choosing a directory records it as a place, newest first.
#[test]
fn places_are_listed_in_the_opened_dialog_and_a_choice_becomes_one() {
    let d = tempfile::tempdir().unwrap();
    let fav = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    std::fs::write(
        cfg.path().join("config"),
        format!("place = {}\n", fav.path().display()),
    )
    .unwrap();

    let mut app = App::new_home(d.path().into(), vec![], 60, 16);
    app.config_dir = Some(cfg.path().into());
    app.launch_dir = Some(d.path().into());
    // Startup prefs load places; drive the same path the binary uses.
    if let Some(h) = app.home_mut() {
        h.places = carrel::config::load_places_in(cfg.path());
    }
    update(&mut app, Action::HomeKey(SearchKey::Cancel)); // -> Normal
    update(&mut app, Action::PickerOpen);
    let h = app.home().unwrap();
    assert!(
        h.picker.roots.contains(&fav.path().to_path_buf()),
        "the remembered place is among the rows: {:?}",
        h.picker.roots,
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
    assert!(places.contains(&fav.path().to_path_buf()));
}

/// The path row's targets must cover the segments they name, edge to edge.
///
/// This is the round trip that matters for the click-first pivot: the
/// painter walks left to right accumulating `x` past separators of its own,
/// and any hit-test that re-derived those offsets would drift. Verified to
/// fail on a one-cell shift of the recorded zone.
#[test]
fn every_path_segment_target_covers_its_own_label() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("docs/today/current")).unwrap();
    let root = d.path().join("docs/today/current");
    let app = App::new_home(root.clone(), vec![], 100, 24);

    let mut painted = carrel::render::Painted::default();
    let mut protocols = std::collections::HashMap::new();
    let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
    t.draw(|f| carrel::render::draw_full(f, &app, &mut painted, &mut protocols))
        .unwrap();
    let buf = t.backend().buffer();

    let crumbs = carrel::home::crumbs(&root);
    let mut seen = 0;
    for target in painted.targets.as_slice() {
        let Action::HomeCrumb(i) = target.action else {
            continue;
        };
        let z = target.zone;
        let text: String = (z.x..z.x + z.w)
            .map(|x| buf[(x, z.y)].symbol().to_string())
            .collect();
        assert_eq!(
            text, crumbs[i].label,
            "segment {i} claims {:?} and covers {text:?}",
            crumbs[i].label
        );
        seen += 1;
    }
    assert_eq!(seen, crumbs.len(), "every segment registers exactly once");

    // And the up arrow sits on its own glyph.
    let up = painted
        .targets
        .as_slice()
        .iter()
        .find(|t| t.action == Action::HomeUp)
        .expect("there is a directory above this one");
    assert_eq!(buf[(up.zone.x, up.zone.y)].symbol(), "\u{2191}");
}

/// A path too long for the terminal drops whole segments from the LEFT and
/// says so with a `…` — the deep end is where you are and where you navigate
/// from, so it is the end that survives.
#[test]
fn a_long_path_elides_from_the_shallow_end() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let d = tempfile::tempdir().unwrap();
    let deep = d
        .path()
        .join("alpha-directory/beta-directory/gamma-directory/delta-directory");
    std::fs::create_dir_all(&deep).unwrap();
    let app = App::new_home(deep, vec![], 40, 24);

    let mut painted = carrel::render::Painted::default();
    let mut protocols = std::collections::HashMap::new();
    let mut t = Terminal::new(TestBackend::new(40, 24)).unwrap();
    t.draw(|f| carrel::render::draw_full(f, &app, &mut painted, &mut protocols))
        .unwrap();
    let buf = t.backend().buffer();

    let row: String = (0..40).map(|x| buf[(x, 7)].symbol().to_string()).collect();
    assert!(row.contains('\u{2026}'), "the cut is marked: {row:?}");
    assert!(
        row.contains("delta-directory"),
        "the directory you are in survives: {row:?}"
    );
    // Nothing painted past the edge, and no target off the row.
    for target in painted.targets.as_slice() {
        assert!(
            target.zone.x + target.zone.w <= 40,
            "{target:?} runs off the terminal"
        );
    }
}
