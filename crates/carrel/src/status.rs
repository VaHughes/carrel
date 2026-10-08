//! What the reader's status row says, and which parts of it are buttons.
//!
//! A pure selector, like [`crate::footer`] and [`crate::breadcrumb`]: it
//! decides *what* goes on the row and what gives way when the row is short,
//! and the painter decides nothing but where the cells are. That split is
//! what lets the fitting rules be tested without a terminal.
//!
//! The row has two ends. The **left** says where you are — the document, and
//! behind it the trail of documents you came through, each one a button back
//! to itself. The **right** says how far through you are and carries the few
//! chips that are about *this* document right now.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::path::Path;

use carrel_core::display_width;

use crate::action::Action;
use crate::app::App;

/// One piece of the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    /// What a click does; `None` is text that only says something.
    pub action: Option<Action>,
    /// What gives way first when the row is short: the lowest goes first.
    rank: u8,
}

impl Chip {
    fn says(text: impl Into<String>, rank: u8) -> Self {
        Self {
            text: text.into(),
            action: None,
            rank,
        }
    }

    fn does(text: impl Into<String>, action: Action, rank: u8) -> Self {
        Self {
            text: text.into(),
            action: Some(action),
            rank,
        }
    }
}

/// Between two chips on the right.
pub const SEP: &str = " · ";
/// Between two documents on the trail.
pub const TRAIL_SEP: &str = " \u{203a} ";
/// Stands for the documents a short row has no room to name.
pub const ELLIPSIS: &str = "\u{2026}";

/// The cells a run of chips takes, separators included.
#[must_use]
pub fn width(chips: &[Chip], sep: &str) -> u16 {
    let text: u16 = chips.iter().map(|c| display_width(&c.text)).sum();
    let seps = u16::try_from(chips.len().saturating_sub(1)).unwrap_or(u16::MAX);
    text.saturating_add(seps.saturating_mul(display_width(sep)))
}

/// The right end, in reading order, before anything is dropped.
///
/// A link the reader has selected — or is pointing at — replaces all of it
/// with where that link goes: the one thing worth knowing before following
/// it, and the reason a terminal shows a URL in its status bar at all.
#[must_use]
pub fn right(app: &App) -> Vec<Chip> {
    if let Some(dest) = app
        .selected_link
        .or_else(|| app.hovered_link())
        .and_then(|id| app.doc.links.get(id.0 as usize))
    {
        let dest: String = dest.chars().filter(|c| !c.is_control()).collect();
        return vec![Chip::says(dest, 9)];
    }
    let mut out = Vec::new();
    // News first: another document was just written, or this one was.
    if let Some(sibling) = &app.sibling {
        let name = sibling.path.file_name().map_or_else(
            || sibling.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let name: String = name.chars().filter(|c| !c.is_control()).collect();
        out.push(Chip::does(
            match sibling.more {
                0 => format!("\u{25cf} {name} changed"),
                n => format!("\u{25cf} {name} changed (+{n})"),
            },
            Action::SiblingOpen,
            4,
        ));
    }
    if !app.changed.is_empty() {
        out.push(Chip::does(
            format!("{} changed", app.changed.len()),
            Action::ChangeStep(1),
            5,
        ));
    }
    let (done, total) = app.task_counts();
    if total > 0 {
        // A plan is a checklist, and how much of it is done is the first
        // thing a reader of one wants. Click: the next task still open.
        out.push(Chip::does(
            format!("{done}/{total} tasks"),
            Action::TaskOpen,
            3,
        ));
    }
    // Percent of the SCROLLABLE range: the bottom of the document must read
    // 100%, or the reader concludes scrolling is broken.
    let max = app.layout.max_scroll(app.text_h());
    let pct = if max == 0 {
        100
    } else {
        (u64::from(app.view.scroll_row) * 100 / u64::from(max)).min(100)
    };
    out.push(Chip::says(format!("{pct}%"), 6));
    // "How much is left" is the question a reader actually has; the
    // percentage answers "where am I". `minutes_left` stays quiet under one.
    if let Some(m) = app.minutes_left() {
        out.push(Chip::says(format!("{m} min left"), 1));
    }
    // A key nobody can see is a feature nobody has (field note, twice).
    out.push(Chip::does("T theme", Action::ThemeCycle, 2));
    // The exit key, visibly: q returns to the file list when one is behind
    // this document, and quits when the file was opened directly.
    out.push(Chip::does(
        if app.home_stash.is_some() {
            "q home"
        } else {
            "q quit"
        },
        Action::CloseFile,
        7,
    ));
    out
}

/// The reader's status row, fitted to its width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Whether the `⌂ ‹ ›` icons at its head are drawn.
    pub icons: bool,
    /// The documents behind this one, oldest first, each painted with a
    /// [`TRAIL_SEP`] after it; a leading [`ELLIPSIS`] chip stands for the
    /// ones there was no room to name.
    pub trail: Vec<Chip>,
    /// The document's name, or the note standing in for it.
    pub left: String,
    /// The right end, in reading order.
    pub right: Vec<Chip>,
}

impl Row {
    /// The cells the row takes. Between the left and a non-empty right end
    /// there is one cell of air.
    #[must_use]
    pub fn width(&self, icons_w: u16) -> u16 {
        let icons = if self.icons { icons_w } else { 0 };
        let sep = display_width(TRAIL_SEP);
        let trail: u16 = self
            .trail
            .iter()
            .map(|c| display_width(&c.text).saturating_add(sep))
            .sum();
        let right = if self.right.is_empty() {
            0
        } else {
            width(&self.right, SEP).saturating_add(1)
        };
        icons
            .saturating_add(trail)
            .saturating_add(display_width(&self.left))
            .saturating_add(right)
    }
}

/// The cells the icons at the head of the row take: `⌂`, and once there is
/// a trail the pair of arrows, each glyph followed by a blank.
#[must_use]
pub fn icons_width(app: &App) -> u16 {
    if app.history.is_empty() && app.future.is_empty() {
        2
    } else {
        6
    }
}

/// The whole row for `room` cells, `left` being the name or the note.
///
/// **One order of giving way, for every part of the row.** The parts used to
/// be fitted one after another — the right end against what the name left,
/// the trail against what the right end left, the icons against the name
/// alone — so when the right end dropped a chip the trail got the cells back
/// and grew, and when the icons went the right end did: the trail's `…`
/// blinked in and out as a window was narrowed, and `q quit` came back
/// after it had gone. Here everything gives way in one sequence, and a
/// narrower row is always somewhere further along it:
///
/// 1. the trail, oldest name first, its `…` standing for them;
/// 2. the `…` itself;
/// 3. a note's trailing clauses, last first (see [`clauses`]) — the
///    afterthought after a `·` is the cheapest thing on the row, and keeping
///    it while the right end empties left a row of blanks once it went;
/// 4. the right end, least important chip first (the last one standing is
///    shortened before it goes — for a link's destination it is the only
///    thing the right end has to say);
/// 5. the icons;
/// 6. and last the name or note itself, cut at a word with an ellipsis
///    ([`shorten`]).
#[must_use]
pub fn row(app: &App, left: &str, room: u16) -> Row {
    let note = app.note.is_some();
    // A one-shot note takes the row for as long as it is up: no trail.
    let trail = if note { Vec::new() } else { trail(app) };
    arrange(left, note, &trail, right(app), icons_width(app), room)
}

/// [`row`], from its parts.
#[must_use]
pub fn arrange(
    left: &str,
    note: bool,
    trail: &[(String, u32)],
    right: Vec<Chip>,
    icons_w: u16,
    room: u16,
) -> Row {
    let clauses = if note { clauses(left) } else { vec![left] };
    let mut clause = 0;
    // The nearest `keep` names of the trail are named.
    let mut keep = trail.len();
    let mut ellipsis = false;
    let mut out = Row {
        icons: true,
        trail: Vec::new(),
        left: String::new(),
        right,
    };
    loop {
        out.left = clauses[clause].to_string();
        out.trail = trail_shown(trail, keep, ellipsis);
        let over = out.width(icons_w) > room;
        if !over {
            return out;
        }
        if keep > 0 {
            keep -= 1;
            ellipsis = true;
        } else if ellipsis {
            ellipsis = false;
        } else if clause + 1 < clauses.len() {
            clause += 1;
        } else if out.right.len() > 1 {
            let weakest = out
                .right
                .iter()
                .enumerate()
                .min_by_key(|(_, c)| c.rank)
                .map_or(0, |(i, _)| i);
            out.right.remove(weakest);
        } else if out.right.len() == 1 {
            // What is left for the last chip, its cell of air included.
            let mut without = out.clone();
            without.right.clear();
            let spare = room
                .saturating_sub(without.width(icons_w))
                .saturating_sub(1);
            match shortened(&out.right[0].text, spare) {
                Some(short) => out.right[0].text = short,
                None => out.right.clear(),
            }
        } else if out.icons {
            out.icons = false;
        } else {
            // Nothing else can give way: the name itself is cut.
            out.left = shorten(&out.left, room);
            return out;
        }
    }
}

/// The trail's chips with the nearest `keep` names, and the `…` before them
/// when `ellipsis`.
fn trail_shown(trail: &[(String, u32)], keep: usize, ellipsis: bool) -> Vec<Chip> {
    let mut out = Vec::new();
    if ellipsis {
        out.push(Chip::says(ELLIPSIS, 0));
    }
    for (label, index) in &trail[trail.len() - keep..] {
        out.push(Chip::does(label.clone(), Action::BackTo(*index), 0));
    }
    out
}

/// The ways a note can be shortened by whole clauses, longest first: the
/// whole note, then each prefix that ends before a [`SEP`] — `reloaded — 7
/// changes · c goes to the next` can lose `· c goes to the next` and still
/// say what happened. An em dash joins a clause to the one before it rather
/// than separating an afterthought, so it is not a place to stop: what
/// follows it is cut at a word instead.
#[must_use]
pub fn clauses(note: &str) -> Vec<&str> {
    let mut out = vec![note];
    let mut ends: Vec<usize> = note.match_indices(SEP).map(|(i, _)| i).collect();
    ends.reverse();
    out.extend(ends.into_iter().filter(|&i| i > 0).map(|i| &note[..i]));
    out
}

/// `text` in at most `room` cells: whole if it fits, otherwise cut after
/// its last word that fits, with an ellipsis to say it was cut. Only when
/// not even the first word fits is it cut inside one — still with the
/// ellipsis, so a reader can always tell a cut from an end.
#[must_use]
pub fn shorten(text: &str, room: u16) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if display_width(text) <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let budget = room - display_width(ELLIPSIS);
    // At a word: before a blank, with any separator the cut leaves dangling
    // (`reloaded —`) taken off too.
    let at_word = text
        .match_indices(' ')
        .map(|(i, _)| {
            text[..i]
                .trim_end_matches(|c: char| c.is_whitespace() || "\u{2014}\u{b7}-:;,".contains(c))
        })
        .filter(|head| !head.is_empty() && display_width(head) <= budget)
        .max_by_key(|head| head.len());
    if let Some(head) = at_word {
        return format!("{head}{ELLIPSIS}");
    }
    let mut used = 0u16;
    let mut end = 0;
    for (at, g) in text.grapheme_indices(true) {
        let w = display_width(g);
        if used.saturating_add(w) > budget {
            break;
        }
        used += w;
        end = at + g.len();
    }
    format!("{}{ELLIPSIS}", &text[..end])
}

/// `text` cut to `room` cells with an ellipsis, or `None` where there is
/// not room for enough of it to mean anything. The head is what is kept:
/// of a destination, the scheme and the host are the part worth reading.
fn shortened(text: &str, room: u16) -> Option<String> {
    const LEAST: u16 = 6;
    if room < LEAST {
        return None;
    }
    let mut out = String::new();
    for c in text.chars() {
        let mut probe = out.clone();
        probe.push(c);
        if display_width(&probe).saturating_add(1) > room {
            break;
        }
        out = probe;
    }
    out.push_str(ELLIPSIS);
    Some(out)
}

/// What a history entry is called on the trail.
fn entry_label(app: &App, path: &Path) -> String {
    if let Some(label) = app.desk_label(path) {
        return label.to_string();
    }
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The documents behind this one, oldest first: `(label, history index)`.
///
/// A run of entries in one document is one name — jumping around inside a
/// file is not visiting it several times — and the index is the run's LAST
/// entry, so the name returns to where that document was left. Entries in
/// the document being read now are in-document jumps, not somewhere else,
/// and are not on the trail at all.
#[must_use]
pub fn trail(app: &App) -> Vec<(String, u32)> {
    let here = app.location_path();
    let mut out: Vec<(String, u32, &Path)> = Vec::new();
    for (i, (path, _)) in app.history.iter().enumerate() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        match out.last_mut() {
            Some(last) if last.2 == path.as_path() => last.1 = i,
            _ => out.push((entry_label(app, path), i, path.as_path())),
        }
    }
    while out
        .last()
        .is_some_and(|last| here.as_deref() == Some(last.2))
    {
        out.pop();
    }
    out.into_iter().map(|(label, i, _)| (label, i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use carrel_core::Document;
    use std::path::PathBuf;

    fn app(src: &str) -> App {
        App::new("here.md".into(), Document::parse(src), 80, 24)
    }

    fn texts(chips: &[Chip]) -> Vec<&str> {
        chips.iter().map(|c| c.text.as_str()).collect()
    }

    /// The right end alone in `room` cells — the cell of air before it is
    /// the row's, not the chips'.
    fn fit(chips: Vec<Chip>, room: u16) -> Vec<Chip> {
        arrange("", false, &[], chips, 0, room.saturating_add(1)).right
    }

    /// The trail alone in `room` cells.
    fn trail_chips(a: &App, room: u16) -> Vec<Chip> {
        arrange("", false, &trail(a), Vec::new(), 0, room).trail
    }

    #[test]
    fn the_right_end_says_how_far_and_how_to_leave() {
        let a = app("short");
        assert_eq!(texts(&right(&a)), ["100%", "T theme", "q quit"]);
    }

    #[test]
    fn a_destination_longer_than_the_room_is_shortened_not_dropped() {
        let url = "https://example.com/a/very/long/path/that/goes/on/and/on/index.html";
        let chips = fit(vec![Chip::says(url, 9)], 30);
        assert_eq!(chips.len(), 1);
        assert_eq!(display_width(&chips[0].text), 30);
        assert!(chips[0].text.starts_with("https://example.com/"));
        assert!(chips[0].text.ends_with(ELLIPSIS));
        assert!(
            fit(vec![Chip::says(url, 9)], 3).is_empty(),
            "no room to mean anything"
        );
        // Several chips still thin by dropping, never by cutting words.
        let a = app("short");
        assert_eq!(texts(&fit(right(&a), 12)), ["q quit"]);
    }

    #[test]
    fn a_checklist_says_how_much_of_it_is_done() {
        let a = app("- [x] one\n- [ ] two\n- [x] three\n");
        let chips = right(&a);
        assert_eq!(chips[0].text, "2/3 tasks");
        assert_eq!(chips[0].action, Some(Action::TaskOpen));
    }

    #[test]
    fn a_short_row_thins_instead_of_going_blank() {
        let mut a = app(&"- [ ] a task\n\nsome words to read here. ".repeat(400));
        a.on_resize(80, 24);
        let all = right(&a);
        assert!(
            texts(&all).iter().any(|t| t.ends_with("min left")),
            "{all:?}"
        );
        let full = width(&all, SEP);

        assert_eq!(fit(all.clone(), full), all, "everything, when it fits");
        let tight = fit(all.clone(), full - 1);
        assert!(
            !texts(&tight).iter().any(|t| t.ends_with("min left")),
            "the estimate goes first: {tight:?}"
        );
        // Order of departure: estimate, theme, tasks, percent, and the way
        // out last of all.
        let mut gone = Vec::new();
        let mut chips = all;
        while !chips.is_empty() {
            let before = texts(&chips)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            chips = fit(chips.clone(), width(&chips, SEP) - 1);
            let after = texts(&chips);
            gone.extend(before.into_iter().filter(|t| !after.contains(&t.as_str())));
        }
        let kind = |t: &str| {
            t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '/' || c == '%' || c == ' ')
                .to_string()
        };
        assert_eq!(
            gone.iter().map(|t| kind(t)).collect::<Vec<_>>(),
            ["min left", "T theme", "tasks", "", "q quit"]
        );
        assert!(fit(right(&a), 0).is_empty());
    }

    #[test]
    fn a_selected_link_says_where_it_goes_and_nothing_else() {
        let mut a = app("see [docs](https://example.com/a)");
        a.selected_link = Some(carrel_core::LinkId(0));
        let chips = right(&a);
        assert_eq!(texts(&chips), ["https://example.com/a"]);
        assert_eq!(chips[0].action, None);
        // A link id the document has outlived says nothing rather than
        // indexing past the end.
        a.selected_link = Some(carrel_core::LinkId(9));
        assert_eq!(texts(&right(&a)), ["100%", "T theme", "q quit"]);
    }

    fn visited(a: &mut App, names: &[&str]) {
        for n in names {
            a.history.push((PathBuf::from(format!("/r/{n}")), 0));
        }
    }

    #[test]
    fn the_trail_names_each_document_once_and_not_the_one_being_read() {
        let mut a = app("x");
        a.file = Some("/r/here.md".into());
        // Two jumps inside README, then PLAN, then two jumps inside here.
        visited(
            &mut a,
            &["README.md", "README.md", "PLAN.md", "here.md", "here.md"],
        );
        assert_eq!(
            trail(&a),
            [("README.md".to_string(), 1), ("PLAN.md".to_string(), 2)],
            "a run is one name, returning to where that document was left"
        );
    }

    #[test]
    fn a_short_row_keeps_the_nearest_documents_and_says_there_are_more() {
        let mut a = app("x");
        a.file = Some("/r/here.md".into());
        visited(&mut a, &["one.md", "two.md", "three.md"]);
        let all = trail_chips(&a, 80);
        assert_eq!(texts(&all), ["one.md", "two.md", "three.md"]);
        assert_eq!(all[2].action, Some(Action::BackTo(2)));

        // three.md + sep = 11; the ellipsis + sep = 4.
        let tight = trail_chips(&a, 15);
        assert_eq!(texts(&tight), [ELLIPSIS, "three.md"]);
        assert_eq!(tight[0].action, None, "the ellipsis is not a button");
        assert_eq!(tight[1].action, Some(Action::BackTo(2)));

        assert_eq!(
            texts(&trail_chips(&a, 5)),
            [ELLIPSIS],
            "only room to say so"
        );
        assert!(trail_chips(&a, 2).is_empty());
        assert!(
            trail_chips(&app("x"), 80).is_empty(),
            "no history, no trail"
        );
    }

    #[test]
    fn whatever_is_kept_fits_the_room_it_was_given() {
        let mut a = app("x");
        a.file = Some("/r/here.md".into());
        visited(
            &mut a,
            &["a-long-name.md", "b.md", "another-long-one.md", "d.md"],
        );
        let sep = display_width(TRAIL_SEP);
        for room in 0..80u16 {
            let chips = trail_chips(&a, room);
            let cells: u16 = chips.iter().map(|c| display_width(&c.text) + sep).sum();
            assert!(
                cells <= room,
                "{cells} cells in a room of {room}: {chips:?}"
            );
        }
    }

    #[test]
    fn a_note_gives_way_by_whole_clauses_after_a_middle_dot() {
        assert_eq!(
            clauses("reloaded \u{2014} 7 changes \u{b7} c goes to the next"),
            [
                "reloaded \u{2014} 7 changes \u{b7} c goes to the next",
                "reloaded \u{2014} 7 changes"
            ]
        );
        assert_eq!(clauses("one \u{b7} two \u{b7} three").len(), 3);
        assert_eq!(clauses("no clauses"), ["no clauses"]);
    }

    #[test]
    fn a_cut_ends_on_a_word_and_says_so() {
        let s = "reloaded \u{2014} 7 changes";
        assert_eq!(shorten(s, 40), s, "whole when it fits");
        assert_eq!(shorten(s, 19), "reloaded \u{2014} 7\u{2026}");
        // The dash the cut would leave dangling goes with it.
        assert_eq!(shorten(s, 12), "reloaded\u{2026}");
        // Not even the first word fits: cut inside it, still marked.
        assert_eq!(shorten(s, 5), "relo\u{2026}");
        assert_eq!(shorten(s, 1), "\u{2026}");
        assert_eq!(shorten(s, 0), "");
        for room in 0..30 {
            assert!(display_width(&shorten(s, room)) <= room, "{room}");
        }
    }

    /// The pure twin of `tests/status_row.rs`: every room from wide to
    /// nothing, and whatever gave way at one room stays gone at every
    /// narrower one.
    #[test]
    fn the_row_only_ever_gives_way_as_its_room_shrinks() {
        let trail: Vec<(String, u32)> = ["the-first.md", "b.md", "a-third-one.md", "four.md"]
            .iter()
            .zip(0..)
            .map(|(n, i)| ((*n).to_string(), i))
            .collect();
        let mut a = app(&"- [ ] a task\n\nsome words to read here. ".repeat(400));
        a.on_resize(80, 24);
        for (left, note) in [
            ("here.md", false),
            (
                "reloaded \u{2014} 7 changes \u{b7} c goes to the next",
                true,
            ),
        ] {
            let mut before: Option<Row> = None;
            let mut ellipsis_runs = 0;
            let mut had = false;
            for room in (0..140u16).rev() {
                let t = if note { &[][..] } else { &trail[..] };
                let r = arrange(left, note, t, right(&a), 6, room);
                assert!(r.width(6) <= room, "room {room}: {r:?}");
                let has = r.trail.first().is_some_and(|c| c.text == ELLIPSIS);
                ellipsis_runs += usize::from(has && !had);
                had = has;
                assert!(ellipsis_runs <= 1, "the \u{2026} came back at {room}");
                if let Some(b) = &before {
                    for c in r.trail.iter().filter(|c| c.action.is_some()) {
                        assert!(b.trail.contains(c), "{c:?} back at {room}");
                    }
                    for c in &r.right {
                        assert!(
                            b.right.iter().any(|d| d.rank == c.rank),
                            "{c:?} back at {room}"
                        );
                    }
                    assert!(b.icons || !r.icons, "icons back at {room}");
                    let stem = r.left.trim_end_matches(ELLIPSIS);
                    assert!(b.left.starts_with(stem), "{:?} then {:?}", b.left, r.left);
                }
                before = Some(r);
            }
        }
    }
}
