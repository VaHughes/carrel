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

/// Drop the least important chips until the rest fit in `room` cells.
///
/// The whole right end used to vanish the moment it did not fit as written.
/// Now it thins: the time estimate goes first, the way out goes last.
#[must_use]
pub fn fit(mut chips: Vec<Chip>, room: u16) -> Vec<Chip> {
    while !chips.is_empty() && width(&chips, SEP) > room {
        let weakest = chips
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| c.rank)
            .map_or(0, |(i, _)| i);
        chips.remove(weakest);
    }
    chips
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

/// The trail's chips for `room` cells: as many of the nearest documents as
/// fit, each with its separator, and an ellipsis for the ones that do not.
///
/// Nearest first, because the document you just left is the one you are most
/// likely to want back. The current document's own name is not here — the
/// caller paints it after these, and it never gives way to them.
#[must_use]
pub fn trail_chips(app: &App, room: u16) -> Vec<Chip> {
    let trail = trail(app);
    let sep = display_width(TRAIL_SEP);
    let ellipsis = display_width(ELLIPSIS).saturating_add(sep);
    let mut used = 0u16;
    let mut kept: Vec<Chip> = Vec::new();
    let mut left_out = false;
    for (n, (label, index)) in trail.iter().enumerate().rev() {
        let cost = display_width(label).saturating_add(sep);
        // If anything older remains after this one, the ellipsis that
        // stands for it has to fit too.
        let reserve = if n > 0 { ellipsis } else { 0 };
        if used.saturating_add(cost).saturating_add(reserve) > room {
            left_out = true;
            break;
        }
        used = used.saturating_add(cost);
        kept.push(Chip::does(label.clone(), Action::BackTo(*index), 0));
    }
    if left_out && used.saturating_add(ellipsis) <= room {
        kept.push(Chip::says(ELLIPSIS, 0));
    }
    kept.reverse();
    kept
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

    #[test]
    fn the_right_end_says_how_far_and_how_to_leave() {
        let a = app("short");
        assert_eq!(texts(&right(&a)), ["100%", "T theme", "q quit"]);
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
}
