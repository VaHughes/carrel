//! Home-screen state: what was found, what is showing, what is selected.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. Every behaviour here is a
//! pure function over a `Vec<Entry>`, so the tests never touch a disk.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::scan::Entry;

/// Width of the lamp splash, and the smallest terminal that gets it.
///
/// These live here rather than in `render.rs` because the *geometry* they
/// decide is shared: paint draws the list under the banner, and hit-testing
/// has to know where that list starts. See [`list_geometry`].
pub const SPLASH_W: u16 = 19;
pub const BANNER_MIN_COLS: u16 = SPLASH_W + 6;
pub const BANNER_MIN_ROWS: u16 = 17;

/// The file list's `(top row, height)` inside a terminal of this size.
///
/// **The ONE derivation of the home screen's list geometry.** `render.rs`
/// builds its `Rect` from this and [`Home::row_at`] inverts it; if either ever
/// computes the offsets itself, clicks land on the wrong file and no frame
/// test can tell — the same trap `App::text_x` exists to close on the reader
/// side.
///
/// Above the list: the banner (or a one-line wordmark on a small terminal)
/// and the active-root line. Below it: the status row, plus the lamplight
/// hint row while it shows.
#[must_use]
pub const fn list_geometry(cols: u16, rows: u16, hints: bool) -> (u16, u16) {
    // Banner: lamp row, 3 shade rows, desk, then the tagline and a blank.
    // Small terminal: just the wordmark. Both are followed by the root line.
    let header = if cols >= BANNER_MIN_COLS && rows >= BANNER_MIN_ROWS {
        7
    } else {
        1
    };
    let top = header + 1;
    let chrome = if hints { 2 } else { 1 };
    let bottom = rows.saturating_sub(chrome);
    (top, bottom.saturating_sub(top))
}

/// Entry rows the picker shows, however many directories matched.
pub const PICKER_ROWS: u16 = 12;

/// The directory picker overlay's box: `(x, y, width, height)`.
///
/// Same contract as [`list_geometry`] and for the same reason — paint centres
/// the box, and [`Home::picker_row_at`] has to invert exactly that centring to
/// turn a click into a directory.
///
/// **The height is fixed and deliberately does not follow the match count.**
/// The box is centred, so a height that tracked the matches would move the
/// input row up and down under the cursor on every keystroke — the matches
/// change with each letter typed. A short list leaves the box part-empty,
/// which is what every other picker of this kind does.
#[must_use]
pub const fn picker_geometry(cols: u16, screen_rows: u16) -> (u16, u16, u16, u16) {
    let width = clamp_u16(cols.saturating_sub(8), 10, 60);
    let height = min_u16(PICKER_ROWS + 3, screen_rows);
    let x = (cols.saturating_sub(width)) / 2;
    let y = (screen_rows.saturating_sub(height)) / 2;
    (x, y, width, height)
}

/// How many directory rows fit in a picker box of this height.
///
/// The box is title · input · entries · a trailing blank, so the entries
/// start at `y + 2` and there are `height - 3` of them. A long match list
/// scrolls inside that window rather than growing the box.
#[must_use]
pub const fn picker_visible(height: u16) -> u16 {
    height.saturating_sub(3)
}

/// The first visible index of a `height`-row window over `len` items.
///
/// **The whole scroll model of the home screen, and it is deliberately not a
/// function of the selection alone.** The old rule was
/// `first = selected - (height - 1)`, which pins the selection to the BOTTOM
/// row the moment the list is taller than the window — so clicking a file
/// halfway down yanked the list until that file sat on the last row. Here the
/// stored offset is honoured and only nudged when the selection would fall
/// outside it, which is what makes a click scroll nothing.
///
/// Paint and hit-testing both call this, so a stale offset (a resize, a batch
/// of scan results) can never make them disagree.
#[must_use]
pub fn window_first(top: usize, selected: usize, len: usize, height: usize) -> usize {
    if height == 0 || len == 0 {
        return 0;
    }
    let mut t = top.min(len.saturating_sub(height));
    if selected < t {
        t = selected;
    } else if selected >= t + height {
        t = selected + 1 - height;
    }
    t
}

const fn min_u16(a: u16, b: u16) -> u16 {
    if a < b { a } else { b }
}

const fn clamp_u16(v: u16, lo: u16, hi: u16) -> u16 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Telescope's model: the screen opens filtering, `Esc` drops to vim keys.
///
/// A picker has to behave like a picker — typing filters — and a vim user
/// pressing `Esc` has to find vim.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HomeMode {
    Filter,
    Normal,
    Picker,
    /// Content search across every scanned file — wave E, Q9. Typing edits
    /// the query; the list shows matching FILES with counts and a context
    /// line; Enter opens at the first match.
    Search,
}

/// The directory picker overlay: an input line and the directories it matches.
///
/// There is no fixed menu of roots any more and no `Other…` row. Typing IS
/// the picker — [`directory_matches`] re-lists on every keystroke — so the
/// places a given machine actually keeps documents are found by looking
/// rather than by guessing at `~/Documents/GitHub`.
#[derive(Clone, Debug, Default)]
pub struct Picker {
    /// Directories matching [`Self::typed`], freshly listed from disk.
    /// Never `$HOME` itself while nothing is typed — see [`directory_matches`].
    pub roots: Vec<PathBuf>,
    pub selected: usize,
    /// The path being typed. Empty means "offer the defaults".
    pub typed: String,
    /// First visible row of `roots`; see [`window_first`].
    pub top: usize,
}

#[derive(Debug)]
pub struct Home {
    pub root: PathBuf,
    /// Newest-modified first, always.
    pub entries: Vec<Entry>,
    /// Indices into `entries` that match `filter`.
    pub filtered: Vec<usize>,
    pub filter: String,
    /// Index into `filtered`.
    pub selected: usize,
    /// First visible index of `filtered`. A real scroll offset, so a click
    /// selects without moving the list under the pointer; see [`window_first`].
    pub top: usize,
    pub mode: HomeMode,
    pub picker: Picker,
    pub scanning: bool,
    pub unreadable: usize,
    /// A one-line note for the status bar, e.g. a vanished saved root.
    pub note: Option<String>,
    /// The content-search query ([`HomeMode::Search`]).
    pub query: String,
    /// Streamed results for `query`. The event loop owns the generation
    /// bookkeeping; only current-generation hits land here.
    pub hits: Vec<crate::grep::Hit>,
    /// Selection into `hits`.
    pub hit_selected: usize,
    /// First visible hit. The `hits` twin of [`Self::top`].
    pub hit_top: usize,
    /// The background grep finished (footer honesty: "searching…" vs count).
    pub grep_done: bool,
    /// Paths the live walk has reported. Lets `finish_scan` drop cache entries
    /// the walk did not rediscover.
    seen: HashSet<PathBuf>,
}

impl Home {
    /// `cached` is painted immediately; the live walk then refines it.
    #[must_use]
    pub fn new(root: PathBuf, cached: Vec<Entry>) -> Self {
        let mut entries = cached;
        entries.sort_by_key(|e| std::cmp::Reverse(e.mtime));
        let mut h = Self {
            root,
            entries,
            filtered: Vec::new(),
            filter: String::new(),
            selected: 0,
            top: 0,
            // The menu greets you; `i` starts filtering (maintainer call,
            // 2026-08-11 — the hints line is the menu, so open on it).
            mode: HomeMode::Normal,
            picker: Picker::default(),
            scanning: true,
            unreadable: 0,
            note: None,
            query: String::new(),
            hits: Vec::new(),
            hit_selected: 0,
            hit_top: 0,
            grep_done: false,
            seen: HashSet::new(),
        };
        h.refilter();
        h
    }

    /// One entry from the live walk. Test convenience — the event loop drains
    /// into [`Self::push_many`], which reconciles a whole batch in one pass.
    pub fn push(&mut self, e: Entry) {
        self.push_many(vec![e]);
    }

    /// Reconcile a batch of walk results in one pass.
    ///
    /// Two distinct jobs here, easily confused:
    ///
    /// - `seen` records that the WALK found these paths, which is what
    ///   [`Self::finish_scan`] uses to drop cache entries that no longer
    ///   exist. Re-reported paths are no-ops.
    /// - replacing an entry already showing from the cache lets the walk's
    ///   fresher mtime win. Without it, every cached file would appear twice.
    ///
    /// One retain + one sort + one refilter per batch, **not per entry** — the
    /// per-entry version was O(N²) across a scan and froze the home screen for
    /// the documented 68,775-file case the async design exists to serve.
    pub fn push_many(&mut self, batch: Vec<Entry>) {
        let mut fresh: Vec<Entry> = Vec::with_capacity(batch.len());
        for e in batch {
            if self.seen.insert(e.path.clone()) {
                fresh.push(e);
            }
        }
        if fresh.is_empty() {
            return;
        }
        let replacing: HashSet<&PathBuf> = fresh.iter().map(|e| &e.path).collect();
        self.entries.retain(|e| !replacing.contains(&e.path));
        self.entries.extend(fresh);
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.mtime));
        self.refilter();
    }

    /// The walk died before finishing. Keep everything — the cached entries
    /// are still the best available answer — and say so.
    pub fn abort_scan(&mut self) {
        self.scanning = false;
        self.note = Some("scan interrupted — showing the cached list".into());
    }

    /// The walk finished. Anything it did not rediscover is gone from disk.
    pub fn finish_scan(&mut self, unreadable: usize) {
        let seen = std::mem::take(&mut self.seen);
        self.entries.retain(|e| seen.contains(&e.path));
        self.seen = seen;
        self.scanning = false;
        self.unreadable = unreadable;
        self.refilter();
    }

    /// Rebuild `filtered` and clamp `selected` so it can never dangle.
    pub fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                needle.is_empty() || e.path.to_string_lossy().to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.top = self.top.min(self.filtered.len().saturating_sub(1));
    }

    /// Nudge the scroll offsets so the selection is on screen, and no further.
    ///
    /// Called once per update with the real list height, which is why
    /// `home_update` is wrapped rather than each arm doing it — an arm that
    /// forgot would leave the selection off screen with no way to tell.
    pub fn clamp_scroll(&mut self, height: usize) {
        if self.mode == HomeMode::Search {
            let visible = (height / HIT_ROWS).max(1);
            self.hit_top = window_first(self.hit_top, self.hit_selected, self.hits.len(), visible);
        } else {
            self.top = window_first(self.top, self.selected, self.filtered.len(), height);
        }
    }

    /// The picker's `(box, first visible entry, visible count)`.
    ///
    /// **The one derivation**, exactly as [`list_geometry`] is for the list:
    /// paint draws from it and [`Self::picker_row_at`] inverts it.
    #[must_use]
    pub fn picker_view(&self, cols: u16, screen_rows: u16) -> ((u16, u16, u16, u16), usize, usize) {
        let g = picker_geometry(cols, screen_rows);
        let visible = usize::from(picker_visible(g.3));
        let first = window_first(
            self.picker.top,
            self.picker.selected,
            self.picker.roots.len(),
            visible,
        );
        (g, first, visible)
    }

    /// The picker's twin of [`Self::clamp_scroll`].
    pub fn clamp_picker_scroll(&mut self, cols: u16, screen_rows: u16) {
        let (_, first, _) = self.picker_view(cols, screen_rows);
        self.picker.top = first;
    }

    /// Which list index the pointer is over, or `None` off the list.
    ///
    /// **The inverse of the paint**, and it must stay that way: both this and
    /// `render.rs` take their window from [`list_geometry`], because a hit-test
    /// that re-derives the geometry drifts from the paint and every click lands
    /// on the wrong row — a failure no frame test can see.
    ///
    /// In [`HomeMode::Search`] a hit occupies TWO rows (the name and its dimmed
    /// context line), so a click on either one selects that hit.
    #[must_use]
    pub fn row_at(&self, row: u16, cols: u16, rows: u16, hints: bool) -> Option<usize> {
        let (top, height) = list_geometry(cols, rows, hints);
        if height == 0 || row < top || row >= top.saturating_add(height) {
            return None;
        }
        let offset = usize::from(row - top);
        let h = usize::from(height);
        if self.mode == HomeMode::Search {
            let visible = (h / HIT_ROWS).max(1);
            let first = window_first(self.hit_top, self.hit_selected, self.hits.len(), visible);
            let i = first + offset / HIT_ROWS;
            (i < self.hits.len()).then_some(i)
        } else {
            let first = window_first(self.top, self.selected, self.filtered.len(), h);
            let i = first + offset;
            (i < self.filtered.len()).then_some(i)
        }
    }

    /// How many directories the picker is offering.
    #[must_use]
    pub fn picker_entries(&self) -> usize {
        self.picker.roots.len()
    }

    /// Which picker entry the pointer is over, or `None` off the entry rows.
    ///
    /// The inverse of [`Self::picker_view`], which paint also uses. Neither
    /// the title row nor the input row is an entry.
    #[must_use]
    pub fn picker_row_at(&self, col: u16, row: u16, cols: u16, screen_rows: u16) -> Option<usize> {
        let ((bx, by, width, _), first, visible) = self.picker_view(cols, screen_rows);
        if col < bx || col >= bx.saturating_add(width) {
            return None;
        }
        // Row `by` is the title and `by + 1` the input; entries start below.
        let top = by.saturating_add(2);
        let vis = u16::try_from(visible).unwrap_or(u16::MAX);
        if row < top || row >= top.saturating_add(vis) {
            return None;
        }
        let i = first + usize::from(row - top);
        (i < self.picker.roots.len()).then_some(i)
    }

    /// Put the selection on an absolute index, clamped to the list.
    pub fn select(&mut self, i: usize) {
        if self.mode == HomeMode::Search {
            if !self.hits.is_empty() {
                self.hit_selected = i.min(self.hits.len() - 1);
            }
        } else if !self.filtered.is_empty() {
            self.selected = i.min(self.filtered.len() - 1);
        }
    }

    /// Move the selection. Saturates rather than wrapping.
    pub fn move_by(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.filtered.len() - 1;
        self.selected = if delta < 0 {
            self.selected.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.selected
                .saturating_add(delta.unsigned_abs() as usize)
                .min(last)
        };
    }

    pub fn go_first(&mut self) {
        self.selected = 0;
    }

    pub fn go_last(&mut self) {
        self.selected = self.filtered.len().saturating_sub(1);
    }

    #[must_use]
    pub fn selected_path(&self) -> Option<&Path> {
        let i = *self.filtered.get(self.selected)?;
        Some(self.entries.get(i)?.path.as_path())
    }

    /// Point at a new root: forget everything and start over.
    ///
    /// The caller restarts the walk; this only resets state.
    pub fn set_root(&mut self, root: PathBuf, cached: Vec<Entry>) {
        self.root = root;
        self.entries = cached;
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.mtime));
        self.seen.clear();
        self.filter.clear();
        self.selected = 0;
        self.top = 0;
        self.scanning = true;
        self.unreadable = 0;
        // Back to the menu, NOT into the filter. Choosing a directory is a
        // "show me what is here" gesture; dropping the user into a filter
        // they did not ask for means the next keystroke silently hides files
        // (maintainer report, 2026-08-21).
        self.mode = HomeMode::Normal;
        self.note = None;
        self.refilter();
    }

    /// Convenience for the tests and the event loop: `directory_matches`
    /// against the current directory.
    #[must_use]
    pub fn matches_for(query: &str) -> Vec<PathBuf> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        directory_matches(query, &cwd)
    }
}

/// Two rows per hit in [`HomeMode::Search`]: the name and its context line.
const HIT_ROWS: usize = 2;

/// A bound on one `read_dir`, not on the list — the picker scrolls.
const MAX_MATCHES: usize = 200;

/// The directories the picker offers for what has been typed.
///
/// The query is read as a **path prefix**, the shell idiom: `~` expands, a
/// trailing `/` lists that directory whole, and anything else matches the
/// last component case-insensitively against its parent's subdirectories. A
/// bare word is resolved against `cwd`, so `wo` finds `./work`.
///
/// # Two rules, both learned the hard way
///
/// **The home directory itself is never offered** while nothing is typed.
/// Scanning all of `~` means descending into every cache, container,
/// virtualenv and mail spool on the machine — the one scan whose cost the
/// 6 ms measurement does not cover. Its *subdirectories* are offered, and
/// someone who genuinely means `~` can type it; it should not be one
/// keystroke away by accident.
///
/// **Nothing is hard-coded.** This replaced a fixed probe for
/// `~/Documents/GitHub` and `~/Documents`, which found nothing at all on a
/// machine that keeps its repositories somewhere else — and every machine
/// keeps them somewhere else. Listing beats guessing.
#[must_use]
pub fn directory_matches(query: &str, cwd: &Path) -> Vec<PathBuf> {
    let q = query.trim();
    if q.is_empty() {
        return default_roots(cwd);
    }
    let (dir, prefix) = split_query(q, cwd);
    list_dirs(&dir, &prefix)
}

/// With nothing typed: where you are, then the top level of your home.
fn default_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut out = vec![cwd.to_path_buf()];
    if let Some(home) = home_dir() {
        out.extend(list_dirs(&home, ""));
    }
    out.retain(|p| p.is_dir());
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// `(directory to list, name prefix to match)` for a typed query.
fn split_query(q: &str, cwd: &Path) -> (PathBuf, String) {
    let expanded = expand_typed(q);
    // The trailing slash has to be read off the STRING: a `PathBuf` drops it,
    // and it is the whole difference between "list this directory" and
    // "match this name in its parent".
    if q.ends_with('/') || q == "~" {
        return (absolutize(expanded, cwd), String::new());
    }
    let prefix = expanded
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = expanded
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    (absolutize(parent, cwd), prefix)
}

/// Expand a typed path for use as a root: `~` becomes `$HOME`.
///
/// Public because `PickerChoose` commits the typed text when nothing matched
/// it, and `~/notes` typed in full has to mean the same directory there as it
/// does in the match list.
#[must_use]
pub fn expand_typed(q: &str) -> PathBuf {
    let Some(rest) = q.strip_prefix('~') else {
        return PathBuf::from(q);
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return PathBuf::from(q); // ~user — not ours to resolve
    }
    let Some(home) = home_dir() else {
        return PathBuf::from(q);
    };
    match rest.strip_prefix('/').filter(|r| !r.is_empty()) {
        Some(r) => home.join(r),
        None => home,
    }
}

fn absolutize(p: PathBuf, cwd: &Path) -> PathBuf {
    if p.as_os_str().is_empty() {
        cwd.to_path_buf()
    } else if p.is_relative() {
        cwd.join(p)
    } else {
        p
    }
}

/// Subdirectories of `dir` whose name starts with `prefix`, sorted.
///
/// Hidden directories stay hidden unless the prefix asks for them — the same
/// rule the shell uses, and the reason a bare `~` does not offer `.cache`.
fn list_dirs(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let needle = prefix.to_lowercase();
    let want_hidden = prefix.starts_with('.');
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            (want_hidden || !name.starts_with('.')) && name.to_lowercase().starts_with(&needle)
        })
        // `is_dir` follows symlinks on purpose: a symlinked project directory
        // is a directory to everyone except `file_type`.
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out.truncate(MAX_MATCHES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn e(name: &str, secs: u64) -> Entry {
        Entry {
            path: PathBuf::from(format!("/root/{name}")),
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
        }
    }

    fn home() -> Home {
        Home::new(
            PathBuf::from("/root"),
            vec![e("alpha.md", 10), e("beta.md", 30), e("docs/gamma.md", 20)],
        )
    }

    // --- click-to-open geometry ---

    /// A terminal big enough for the banner, with the hint row showing.
    const BIG: (u16, u16, bool) = (100, 40, true);

    #[test]
    fn the_list_starts_under_the_banner_and_stops_above_the_chrome() {
        let (top, h) = list_geometry(100, 40, true);
        assert_eq!(top, 8, "7 banner rows + the root line");
        assert_eq!(h, 40 - 2 - 8, "status + hint row below");
        // Hiding the hints gives the list one more row.
        assert_eq!(list_geometry(100, 40, false).1, h + 1);
        // Too small for the banner: just the wordmark and the root line.
        assert_eq!(list_geometry(40, 10, true).0, 2);
    }

    #[test]
    fn a_click_on_the_first_row_selects_the_first_file() {
        let h = home();
        let (top, _) = list_geometry(BIG.0, BIG.1, BIG.2);
        assert_eq!(h.row_at(top, BIG.0, BIG.1, BIG.2), Some(0));
        assert_eq!(h.row_at(top + 1, BIG.0, BIG.1, BIG.2), Some(1));
        assert_eq!(h.row_at(top + 2, BIG.0, BIG.1, BIG.2), Some(2));
    }

    #[test]
    fn clicks_off_the_list_are_not_hits() {
        let h = home();
        let (top, height) = list_geometry(BIG.0, BIG.1, BIG.2);
        assert_eq!(h.row_at(top - 1, BIG.0, BIG.1, BIG.2), None, "the banner");
        assert_eq!(
            h.row_at(top + height, BIG.0, BIG.1, BIG.2),
            None,
            "the status row"
        );
        assert_eq!(
            h.row_at(top + 3, BIG.0, BIG.1, BIG.2),
            None,
            "past the last of three files"
        );
    }

    #[test]
    fn an_empty_list_has_nothing_to_click() {
        let mut h = home();
        h.filter = "no-such-file".into();
        h.refilter();
        let (top, _) = list_geometry(BIG.0, BIG.1, BIG.2);
        assert_eq!(h.row_at(top, BIG.0, BIG.1, BIG.2), None);
    }

    #[test]
    fn a_scrolled_window_maps_rows_to_the_files_actually_shown() {
        // A list taller than the viewport: paint windows on the stored scroll
        // offset, and the hit-test has to follow it rather than assuming
        // row 0 == item 0.
        let many: Vec<Entry> = (0..50).map(|i| e(&format!("f{i}.md"), i)).collect();
        let mut h = Home::new(PathBuf::from("/root"), many);
        let (cols, rows, hints) = (100u16, 14u16, true);
        let (top, height) = list_geometry(cols, rows, hints);
        h.selected = h.filtered.len() - 1; // scrolled to the bottom
        h.clamp_scroll(usize::from(height));
        let first = h.selected - (usize::from(height) - 1);
        assert_eq!(h.top, first);
        assert_eq!(h.row_at(top, cols, rows, hints), Some(first));
        assert_eq!(
            h.row_at(top + height - 1, cols, rows, hints),
            Some(h.selected),
            "the last row is the selected one when scrolled to the end"
        );
    }

    /// The maintainer's report, 2026-08-21: scroll down, click a file, and
    /// the list jumped so the clicked file sat on the bottom row. Selecting
    /// something already on screen must move NOTHING.
    #[test]
    fn clicking_a_visible_file_scrolls_the_list_by_nothing() {
        let many: Vec<Entry> = (0..50).map(|i| e(&format!("f{i}.md"), i)).collect();
        let mut h = Home::new(PathBuf::from("/root"), many);
        let (cols, rows, hints) = (100u16, 14u16, true);
        let (top, height) = list_geometry(cols, rows, hints);
        let hh = usize::from(height);

        // Scroll down past a screenful, the way the wheel does.
        for _ in 0..6 {
            h.move_by(3);
            h.clamp_scroll(hh);
        }
        let before = h.top;
        assert!(before > 0, "the list really did scroll");

        // Click the row under the pointer, wherever it is in the window.
        let clicked = h.row_at(top + 2, cols, rows, hints).unwrap();
        h.select(clicked);
        h.clamp_scroll(hh);
        assert_eq!(h.top, before, "the window must not move under the pointer");
        assert_eq!(h.selected, clicked, "and the click still lands");

        // The same file is still painted on the same row afterwards.
        assert_eq!(h.row_at(top + 2, cols, rows, hints), Some(clicked));
    }

    #[test]
    fn the_window_follows_the_selection_off_either_edge() {
        assert_eq!(window_first(0, 0, 50, 10), 0);
        assert_eq!(window_first(0, 9, 50, 10), 0, "still on screen");
        assert_eq!(window_first(0, 10, 50, 10), 1, "one row down, not a jump");
        assert_eq!(window_first(20, 15, 50, 10), 15, "scrolled back up to it");
        assert_eq!(window_first(45, 49, 50, 10), 40, "never past the end");
        assert_eq!(window_first(9, 0, 0, 10), 0, "an empty list");
        assert_eq!(window_first(9, 3, 50, 0), 0, "no room at all");
    }

    #[test]
    fn in_search_mode_a_hit_owns_two_rows() {
        let mut h = home();
        h.mode = HomeMode::Search;
        h.hits = vec![
            crate::grep::Hit {
                path: PathBuf::from("/root/alpha.md"),
                count: 2,
                first_line: "a".into(),
            },
            crate::grep::Hit {
                path: PathBuf::from("/root/beta.md"),
                count: 1,
                first_line: "b".into(),
            },
        ];
        let (top, _) = list_geometry(BIG.0, BIG.1, BIG.2);
        assert_eq!(h.row_at(top, BIG.0, BIG.1, BIG.2), Some(0), "name row");
        assert_eq!(
            h.row_at(top + 1, BIG.0, BIG.1, BIG.2),
            Some(0),
            "its context line selects the SAME hit"
        );
        assert_eq!(h.row_at(top + 2, BIG.0, BIG.1, BIG.2), Some(1));
        assert_eq!(h.row_at(top + 4, BIG.0, BIG.1, BIG.2), None, "past the end");
    }

    #[test]
    fn select_clamps_and_respects_the_mode() {
        let mut h = home();
        h.select(2);
        assert_eq!(h.selected, 2);
        h.select(99);
        assert_eq!(h.selected, 2, "clamped to the last file");
        h.mode = HomeMode::Search;
        h.hits = vec![crate::grep::Hit {
            path: PathBuf::from("/root/alpha.md"),
            count: 1,
            first_line: String::new(),
        }];
        h.select(0);
        assert_eq!(h.hit_selected, 0);
        assert_eq!(
            h.selected, 2,
            "the file selection is untouched in search mode"
        );
    }

    #[test]
    fn a_click_in_the_picker_lands_on_the_row_under_it() {
        let mut h = home();
        h.mode = HomeMode::Picker;
        h.picker.roots = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let (cols, rows) = (80u16, 24u16);
        let ((x, y, w, _), _, _) = h.picker_view(cols, rows);

        assert_eq!(h.picker_row_at(x + 1, y, cols, rows), None, "the title row");
        assert_eq!(
            h.picker_row_at(x + 1, y + 1, cols, rows),
            None,
            "the input row"
        );
        assert_eq!(h.picker_row_at(x + 1, y + 2, cols, rows), Some(0));
        assert_eq!(h.picker_row_at(x + 1, y + 3, cols, rows), Some(1));
        assert_eq!(h.picker_row_at(x + 1, y + 4, cols, rows), None, "past it");
        assert_eq!(
            h.picker_row_at(x - 1, y + 2, cols, rows),
            None,
            "left of the box"
        );
        assert_eq!(
            h.picker_row_at(x + w, y + 2, cols, rows),
            None,
            "right of the box"
        );
    }

    #[test]
    fn a_match_list_taller_than_the_screen_scrolls_inside_the_box() {
        let mut h = home();
        h.mode = HomeMode::Picker;
        h.picker.roots = (0..100).map(|i| PathBuf::from(format!("/d{i}"))).collect();
        let (cols, rows) = (80u16, 12u16);
        let ((_, y, _, height), _, visible) = h.picker_view(cols, rows);
        assert_eq!(height, rows, "the box stops at the screen");
        assert_eq!(visible, usize::from(rows) - 3);

        h.picker.selected = 40;
        h.clamp_picker_scroll(cols, rows);
        let ((_, _, _, _), first, _) = h.picker_view(cols, rows);
        assert_eq!(first, 40 + 1 - visible);
        assert_eq!(
            h.picker_row_at(cols / 2, y + 2, cols, rows),
            Some(first),
            "the top entry row is the first VISIBLE match"
        );

        // And the box does NOT shrink when the match list does: a height
        // that followed the matches would move the input row under the
        // cursor on every keystroke.
        let tall = h.picker_view(cols, 40).0.3;
        h.picker.roots.truncate(1);
        assert_eq!(h.picker_view(cols, 40).0.3, tall, "the box height is fixed");
    }

    #[test]
    fn entries_are_sorted_newest_first() {
        let h = home();
        let names: Vec<_> = h.entries.iter().map(|x| x.path.to_str().unwrap()).collect();
        assert_eq!(
            names,
            ["/root/beta.md", "/root/docs/gamma.md", "/root/alpha.md"]
        );
    }

    #[test]
    fn an_empty_filter_shows_everything() {
        assert_eq!(home().filtered.len(), 3);
    }

    #[test]
    fn filtering_is_a_case_insensitive_substring_on_the_path() {
        let mut h = home();
        h.filter = "GAM".into();
        h.refilter();
        assert_eq!(h.filtered.len(), 1);
        assert!(h.selected_path().unwrap().ends_with("gamma.md"));
    }

    #[test]
    fn selection_is_clamped_when_the_filter_shrinks_the_list() {
        let mut h = home();
        h.move_by(2);
        assert_eq!(h.selected, 2);
        h.filter = "beta".into();
        h.refilter();
        assert_eq!(h.selected, 0, "selection must not dangle past the list");
    }

    #[test]
    fn movement_saturates_rather_than_wrapping() {
        let mut h = home();
        h.move_by(-5);
        assert_eq!(h.selected, 0);
        h.move_by(50);
        assert_eq!(h.selected, 2);
    }

    #[test]
    fn an_entry_arriving_during_a_scan_joins_in_sorted_order() {
        let mut h = home();
        h.push(e("newest.md", 99));
        assert!(h.entries[0].path.ends_with("newest.md"));
        assert_eq!(h.filtered.len(), 4);
    }

    #[test]
    fn a_duplicate_from_the_cache_is_not_shown_twice() {
        // The cache is painted first, then the live walk rediscovers the same
        // files. Without dedup every entry would appear twice.
        let mut h = home();
        h.push(e("alpha.md", 10));
        assert_eq!(h.entries.len(), 3, "{:?}", h.entries);
    }

    #[test]
    fn finishing_a_scan_drops_cached_entries_the_walk_did_not_rediscover() {
        let mut h = Home::new(PathBuf::from("/root"), vec![e("stale.md", 1)]);
        h.push(e("real.md", 2));
        h.finish_scan(0);
        let names: Vec<_> = h.entries.iter().map(|x| x.path.to_str().unwrap()).collect();
        assert_eq!(names, ["/root/real.md"], "the stale cache entry must go");
        assert!(!h.scanning);
    }

    #[test]
    fn selected_path_is_none_when_nothing_matches() {
        let mut h = home();
        h.filter = "zzzz".into();
        h.refilter();
        assert!(h.selected_path().is_none());
    }

    #[test]
    fn changing_root_forgets_the_previous_directory_entirely() {
        let mut h = home();
        h.filter = "alpha".into();
        h.refilter();
        h.push(e("alpha.md", 10));
        h.set_root(PathBuf::from("/other"), vec![e("new.md", 5)]);
        assert_eq!(h.root, PathBuf::from("/other"));
        assert_eq!(h.entries.len(), 1);
        assert!(
            h.filter.is_empty(),
            "a stale filter would hide the new root"
        );
        assert_eq!(
            h.mode,
            HomeMode::Normal,
            "choosing a directory shows what is there; it does not start a filter"
        );
        assert!(h.scanning);
        // The old root's paths must not survive into finish_scan.
        h.finish_scan(0);
        assert!(h.entries.is_empty(), "nothing was rediscovered");
    }

    #[test]
    fn a_batch_of_entries_reconciles_in_one_pass() {
        let mut h = Home::new(PathBuf::from("/root"), vec![e("cached.md", 5)]);
        let batch: Vec<Entry> = (0..1000).map(|i| e(&format!("f{i}.md"), i)).collect();
        h.push_many(batch);
        assert_eq!(h.entries.len(), 1001);
        // Sorted newest-first throughout.
        for w in h.entries.windows(2) {
            assert!(w[0].mtime >= w[1].mtime);
        }
        // A second batch with duplicates must not double anything.
        h.push_many(vec![e("f1.md", 1), e("new.md", 2000)]);
        assert_eq!(h.entries.len(), 1002);
        assert!(h.entries[0].path.ends_with("new.md"));
    }

    #[test]
    fn an_aborted_scan_keeps_the_cached_entries() {
        // A crashed walker must NOT wipe the list down to what it managed to
        // report — the cache is still the best available answer.
        let mut h = Home::new(
            PathBuf::from("/root"),
            vec![e("a.md", 1), e("b.md", 2), e("c.md", 3)],
        );
        h.push(e("a.md", 1)); // the walk got this far, then died
        h.abort_scan();
        assert_eq!(
            h.entries.len(),
            3,
            "cached entries survive: {:?}",
            h.entries
        );
        assert!(!h.scanning);
        assert!(h.note.is_some(), "the interruption is surfaced, not hidden");
    }

    // --- the directory picker's path completion ---

    /// A directory tree to complete against: `<tmp>/{work,workshop,.hidden,note.md}`.
    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for sub in ["work", "workshop", ".hidden"] {
            std::fs::create_dir(d.path().join(sub)).unwrap();
        }
        std::fs::write(d.path().join("note.md"), "x").unwrap();
        d
    }

    #[test]
    fn nothing_typed_offers_the_current_directory_first() {
        let d = tree();
        let roots = directory_matches("", d.path());
        assert_eq!(roots.first(), Some(&d.path().to_path_buf()));
        assert!(roots.iter().all(|r| r.is_dir()), "{roots:?}");
    }

    #[test]
    fn the_home_directory_itself_is_never_offered() {
        // Scanning all of ~ descends into every cache, container and virtualenv
        // on the machine. It must not be one keystroke away by accident;
        // typing `~` is there for anyone who really means it.
        let Some(home) = home_dir() else { return };
        let roots = directory_matches("", Path::new("/tmp"));
        assert!(!roots.contains(&home), "offered $HOME: {roots:?}");
    }

    #[test]
    fn nothing_typed_offers_the_home_directorys_subdirectories() {
        // The replacement for the hard-coded ~/Documents/GitHub probe, which
        // found nothing at all on a machine that keeps repositories elsewhere.
        let Some(home) = home_dir() else { return };
        let Some(first_sub) = list_dirs(&home, "").into_iter().next() else {
            return; // a home with no visible subdirectory: nothing to assert
        };
        let roots = directory_matches("", Path::new("/tmp"));
        assert!(roots.contains(&first_sub), "{roots:?}");
    }

    #[test]
    fn a_typed_prefix_lists_the_directories_it_matches() {
        let d = tree();
        let q = d.path().join("wor");
        let roots = directory_matches(q.to_str().unwrap(), Path::new("/"));
        assert_eq!(
            roots,
            vec![d.path().join("work"), d.path().join("workshop")],
            "both prefixes match, sorted",
        );
        // Files are not directories, and the match is case-insensitive.
        let q = d.path().join("NOTE");
        assert!(
            directory_matches(q.to_str().unwrap(), Path::new("/")).is_empty(),
            "a file is not a root",
        );
        let q = d.path().join("WORKS");
        assert_eq!(
            directory_matches(q.to_str().unwrap(), Path::new("/")),
            vec![d.path().join("workshop")],
        );
    }

    #[test]
    fn a_trailing_slash_lists_the_directory_whole() {
        let d = tree();
        let q = format!("{}/", d.path().display());
        assert_eq!(
            directory_matches(&q, Path::new("/")),
            vec![d.path().join("work"), d.path().join("workshop")],
            "the hidden one stays hidden",
        );
    }

    #[test]
    fn a_hidden_directory_appears_only_when_the_prefix_asks_for_it() {
        let d = tree();
        let q = d.path().join(".hid");
        assert_eq!(
            directory_matches(q.to_str().unwrap(), Path::new("/")),
            vec![d.path().join(".hidden")],
        );
    }

    #[test]
    fn a_bare_word_completes_against_the_current_directory() {
        let d = tree();
        assert_eq!(
            directory_matches("work", d.path()),
            vec![d.path().join("work"), d.path().join("workshop")],
        );
    }

    #[test]
    fn a_tilde_expands_to_the_home_directory() {
        let Some(home) = home_dir() else { return };
        assert_eq!(expand_typed("~"), home);
        assert_eq!(expand_typed("~/notes"), home.join("notes"));
        // `~user` is the shell's job, not ours — it must stay literal rather
        // than silently resolving to the wrong person's home.
        assert_eq!(expand_typed("~root/x"), PathBuf::from("~root/x"));
        assert_eq!(
            directory_matches("~", Path::new("/tmp")),
            list_dirs(&home, "")
        );
    }

    #[test]
    fn a_query_that_matches_nothing_is_an_empty_list() {
        let d = tree();
        let q = d.path().join("zzz");
        assert!(directory_matches(q.to_str().unwrap(), Path::new("/")).is_empty());
        assert!(directory_matches("/no/such/place/at/all", Path::new("/")).is_empty());
    }
}
