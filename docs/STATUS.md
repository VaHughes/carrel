# Status

Last updated: 2026-10-21

## Current state

- **Latest release: v2026.10.21** (2026-10-21), the first published by
  `.github/workflows/publish.yml`: crates.io (both crates), the Homebrew tap, COPR
  (2026.10.21-1), the `.deb`s, and the checksum stamp with real nix hashes
  (`38f19d0`). Its GitHub Release was created by hand from the run's verified
  artifacts after `release.yml`'s `--target` hit a 403 (fixed on `main`, unproven until
  the next tag). Its nix build fails one test that assumed `/tmp`; fixed on `main`, so
  the nixpkgs PR should wait for the next release.
- Previous: v2026.10.8 (2026-10-08) — the crash on closing the terminal fixed, and the
  reader-beside-an-agent slate.
- All five local gates passed on 2026-10-08: **1,173 tests**, zero-warning clippy,
  formatting, architectural discipline and packaging. Color-sensitive PTY tests need
  `NO_COLOR` unset in this shell.
- **Decisions made 2026-10-08:** a click outside an open pane closes it and does nothing
  else, the status-row chips included; `≡` always works, replacing whatever pane is up;
  a key that opens a pane replaces one that takes no typed text; a note's trailing
  clause outlasts the `T theme` and time-left chips.
- **Decisions made 2026-09-30:** `carrel --latest` means the current directory (or the
  folder named), never the folder saved for the file list; a `[^mark]` with no footnote
  is plain text, not a button; search results keep `Back`, like the tags page; duplicate
  heading fragments are numbered GitHub's way (until free). Design record:
  `2026-09-30-reader-beside-an-agent-design.md` in the notes repo.
- Feature-complete for the terminal reader as planned; the roadmap in `README.md` is the
  authoritative done/open list. What remains on it is packaging (AUR, nixpkgs, `.deb`)
  and the GUI.

## Recently completed

- Unreleased (implemented 2026-10-19/20) — **the open list, cleared, then three cold
  reviews and two fix waves.** Built in parallel worktrees, merged, then reviewed by two
  code readers and one tester who only drove the binary. Shipped to `main`: the targeted
  footnote menu jump; every defect the old Open list named; inline and nested `#tags`;
  hyphenation in ten languages with a marked cut for over-long tokens; the whole
  footnote in its box; smart-punctuation search with home counts taken from the reader's
  own search; `git log` records and hostile-message escaping; `.deb` packaging; the
  exact `carrel-core` pin; `THIRD-PARTY-NOTICES` and an honest license expression.
  *What review found (all fixed):* a debug-build overflow panic on long soft-hyphen
  ladders; runs wider than 65,535 cells collapsing into one row (widths are `u32` now);
  quadratic hyphenation of long runs; a per-hit row walk that froze the reader on
  100,000 hits (`cols_for_doc_ranges`); commit messages that could still forge headings
  with tab-separated markers; German and Norwegian pattern files carrying non-pattern
  lines; a `.deb` guard that passed a package setting a default handler. The two flaky
  pty tests (`the_path_row_walks_the_directory_tree`, `live_resize_…`) raced input
  against the app; they now wait for the effect, 20/20 under full load.
  Gates: **1,306 tests**, zero-warning clippy, fmt, discipline, packaging.
  Q3, Q5, Q29, Q33 and Q35 are answered in the private notes' `research.md`.

- 2026.10.8 (implemented 2026-10-08) — **the known-defects list, cleared, and what three rounds
  of review found around it.** Fixed:
  - *Crash on close (since 2026.9.1):* closing a terminal aborted carrel with a core dump —
    `ratatui::restore` reports failure with `eprintln!`, which panics on the dead
    terminal, and `panic = "abort"`. Restores are silent (`try_restore`, own panic hook
    in `init_terminal`, `say!` for stderr), and a `watch_for_hangup` thread ends a reader
    whose terminal is gone without a SIGHUP — crossterm 0.29's `try_read` retries an
    EOF/EIO read forever inside `event::poll`.
  - *Piped input (since 2026-09-30):* `poll_stream` asked `showing_pipe()` while the
    buffer was taken out of `App::piped`, so nothing piped was drawn; the pty tests
    passed on `script`'s typescript header, which echoes the command (now stripped by
    `captured()`).
  - *One pane at a time:* `App::pane()` is the one answer; openers replace a pane that
    takes no typed text; a full-frame `PaneClose` scrim (`Z_SCRIM`) makes a click outside
    a pane close it and nothing else; `≡` and the lamp stay live; panes never cover the
    status row (`App::pane_rows`); border key hints are buttons.
  - *Frontmatter:* aligned in `Document::text` at parse with one `value_col`, wrapped by
    `meta_fit`, painted through `paint_row`; only a block at the start of the file
    (`carrel_core::frontmatter`, shared with the tag and title scans) counts;
    `meta_split(line, MetaFormat)`; `Document::copy_text` gives back the source.
  - *Widths:* code, math and images are laid out at the width they are painted
    (`layout::block_width`, `measure_inset`); tables use the bleed clear of the margin
    outline.
  - *Smaller:* text after a nested block in a tight item; the status row's one order of
    giving way and word cuts; margin marks sharing cells (`theme::bar_cut`); menus offer
    only rows that can act; a note draft is never dropped by leaving; footnote marks in
    code spans.
- 2026.10.8 (implemented 2026-09-30) — **for the reader beside an agent: 25 additions**.
  *Arriving:* `carrel --latest`, `FILE:42` / `FILE#heading`, "did you mean" for a
  mistyped option or file. *Seeing what changed:* a reload marks changed blocks
  (`c` steps through them), a status chip when another document is written, a
  `7/12 tasks` chip. *Taking something away:* copy `path:line`, a section link, a
  quote signed with its source, every note at once, and a `copy` chip under every
  code block. *Knowing where you are:* `Forward`, a trail of the documents behind
  you on the status row, `Back` from search results, a link's destination on hover,
  dead links struck through, a footnote shown where its mark is, scrollbar notches
  at sections. *The file list:* how far through and how long ago on every row,
  three orders on `s`, a preview pane at 100 columns, a pasted or dropped path
  opens. *Also:* tags on the frontmatter card are buttons and `#` works inside a
  document, help rows run what they describe, collapsed sections are remembered
  per document. New pure modules `cli.rs`, `status.rs`, `peek.rs` (rule-6 listed).
- 2026.10.8 (implemented 2026-09-30) — **defects that slate turned up**. Four older ones found
  while building it: `#L42` and a search hit's line meant visual row 42, not line
  42 of the file; `[[wikilinks]]` never resolved for a document named on the
  command line; four help descriptions lost their last letter; and a click's
  mouse-up wiped the note its own press produced. Then three independent reviewers
  read the finished diff cold and found more, all fixed with a reproduction each
  (`tests/review_nav.rs` and neighbors): a panic when a pipe's next chunk landed
  behind the tags page; a multi-second freeze reloading a document with hundreds of
  collapsed sections; a button's release copying a stale selection over what the
  button copied; reload marks blind to a ticked task; `c` stuck in the last
  screenful; wrong lines for table cells; links wrongly called dead (and paths
  outside the folder being `stat`ed to decide); the highlight moving to another
  file under "recently read"; a pasted paragraph freezing the filter; a
  future-dated file announced forever; help rows for `q`/`Q` that lit and did
  nothing; and hover lighting buttons through an open pane. Two were older and
  only exposed now: the reloader took its baseline at its first look rather than
  at load, so a file rewritten within a second of opening was never noticed; and
  a jump inside a piped document pushed an empty path onto the history.
  **Known defect, older, not fixed here:** the frontmatter card paints each
  source line clipped while the layout counts it wrapped, so in a narrow window a
  long `tags:` line runs off the edge (its far tags cannot be clicked) and blank
  rows follow the card. The card's value column is padding that is not in the
  text, which is why this is not a one-line change.
  **Known limits:** a
  link to `other.md#heading` is judged by the file, not the heading; reload marks
  compare rendered blocks, so a change that renders identically is not marked;
  the sibling watch walks the document's folder tree (it rests 25× as long as a
  walk takes, so a large tree is checked rarely rather than expensively).

- 2026.10.8 (implemented 2026-09-30) — **hyphenation at narrow measures**. Below 70 cells of
  text, a long word that misses a row end is divided by TeX's American English
  patterns (`carrel-core/src/hyphen/`, no dependency; identical to an independent
  implementation over 48k words). The hyphen is a flag on the row and a painted dim
  cell, never text: search, selection, copy, `--plain` and `--render` are unchanged.
  English only, decided per document (`lang:` in frontmatter, else a function-word
  test calibrated against thirteen other languages); `hyphenate = false` or the
  settings pane turns it off. +12% on the height pass at 40 columns, nothing at 80.
- 2026.10.8 (implemented 2026-09-30) — **tags**. `#` on the home screen reads every document's
  frontmatter tags on a background thread and opens them as a generated page: a
  section per tag, a link per document, collapsed into a tag list when long. It is
  kept as a desk, so `Backspace` from a document opened there returns to the page as
  it was left. No index on disk.
- 2026.10.8 (implemented 2026-09-30) — **three older defects, found by the tests written for the
  above and by an independent review of the diff**. (1) Every opener but the first
  built a plain layout of its own — wrong math heights and the columns table mode
  reset until the next resize; all four now end in `relayout()`. (2) Link
  destinations were never percent-decoded, and search-results links to files with
  `#`, `:` or `?` in their names could not open; `links::encode_target` /
  `resolve_local`. (3) `--tutorial` read the config after laying out.
  Verification: **950 tests**, all five gates, all-target and MSRV checks; real PTY
  screens inspected for hyphenation at 30–40 columns and for the tags page (expand,
  open, back) on a 1,834-document tree.

- 2026.10.8 follow-up (implemented 2026-09-29) — **search tails, navigation, and resize safety**.
  Reader and home searches reserve room for counts and keep the query tail visible.
  Selected outline/bookmark/link entries wrap; clipped paths retain their suffix,
  with grapheme-safe elision. A shared feasibility guard prevents hidden choices,
  note edits, and paste while a pane cannot fit. Both event loops check dimensions
  before input, discard stale pointer geometry, and invalidate the retained screen
  after resize activity. The live PTY test caught and now guards immediate Enter
  saving an invisible note and shrink/grow bursts leaving a partly blank menu.
  Verification: **864 tests**, all five gates, all-target checks, and debug build
  passed. Actual 24-column search/note/menu screens inspected; real SIGWINCH bursts,
  selection copying, note persistence, and terminal restoration tested through both
  direct-open and home-screen event loops. Python 3 is required for the resize driver.

- 2026.10.8 follow-up (implemented 2026-09-29) — **complete settings entries and pane wheels**.
  Long labels and values wrap instead of colliding; the selected entry fits as a
  whole, with navigation buttons and a compact footer. In short panes the config
  path gives way to entries. Wheel events use the keyboard's existing arrow
  dispatcher for settings, help, outline, bookmarks, and link panes, preventing
  document movement underneath; this also fixes help scrolling on home.
  Verification: 858 tests and all five gates passed, along with all-target checks;
  debug binary rebuilt and settings inspected in real 24×6 and 12×6 PTYs.

- 2026.10.8 (implemented 2026-09-29) — **small-terminal layouts**. Reading margins and chrome
  adapt without overwriting preferences (40×6 now has five reading rows instead of
  one); the menu launcher remains visible. Compact panes use the window, help wraps,
  settings selections stay visible, information cards wrap and scroll, and close
  controls survive tiny sizes. Menus scroll by keys, wheel, or buttons while hover
  preserves the window. Home prioritizes file rows and keeps resume hit-testing honest.
  Adversarial checks cover resize thresholds, zero-sized frames, Unicode pointer
  inversion, search/anchor preservation, actual close actions, and menu mouse dispatch.
  Verification: 856 tests passed; clippy, formatting, architecture, packaging, and
  all-target checks passed. The local debug binary was rebuilt and real PTY screens
  inspected at 40×6, 32×8, and 24×6/8.
  Hyphenation remains separate: language selection and synthetic-glyph copy/search
  behavior require their own design; no document-text or core layout changes here.

- 2026.9.23 (implemented 2026-09-22) — **image lightbox and marginalia**. Images open full-screen
  from a click, Enter at the image, or the Images menu; previous/next controls and
  close preserve reading position. Highlights (`v`) and notes (`a`) persist outside
  the document, reattach by quote/context, and retain unresolved text after edits.
  `V` reviews/edits/deletes/exports notes and `M` walks them. The editor supports
  multiline Unicode and bracketed paste; failed writes keep the draft. Piped notes
  survive link round trips and export via OSC 52.

- 2026.9.23 (implemented 2026-09-22) — **word-level diff highlighting and horizontal table
  scrolling**. Adjacent removed/added runs get bounded Unicode word comparison,
  with stronger semantic tokens on changed words and punctuation; search and
  selection still override their styles. Columns view (`t`) preserves whole table
  rows; `Shift-←` / `Shift-→`, per-table buttons and the context menu pan across
  them. Search reveals hidden columns, resize keeps a visible match in view, and
  paint/pointer inversion share grapheme clipping. Cards remain the default.

- 2026.9.23 (implemented 2026-09-21) — **the beginner slate**, from an audit of what a
  first-time reader meets. Vocabulary: one word per idea (collapse/expand,
  folder, heading bar, hint row, bookmarks, focus, drawn/text, cards/columns,
  keep up), the vim word `normal` gone from the home status bar, `gg` gone from
  the resume note, `h help` not `h more`, `↑/↓ scroll` not `j/k scroll`. Keys:
  Backspace/`←` back, `→` follow, `+`/`-` text width, PageUp/PageDown on home,
  F3 find-next — all previously unbound, so vim loses nothing; Ctrl-C copies a
  selection instead of quitting; Ctrl-G and Ctrl-Z stop swallowing the next
  keystroke. Honesty: the clipboard, the image protocol, non-markdown files,
  empty documents, a too-small help window, and human error messages. New: a
  settings pane (`,`) listing all nine preferences with their values and the
  config path, and `carrel --tutorial`.

  **Deliberately not done:** `?` still opens a backward search and `Ctrl-F`
  still pages down. Both are the vim/less meanings and rebinding them would
  take something away; the click routes to help and search are the answer
  instead, and both are now plainly labelled.
- 2026.9.23 (implemented 2026-09-16 / 09-21) — the library browser replacing the path-prompt picker,
  search results opening as a generated document, clickable breadcrumb segments, a help
  sheet that filters as you type, and painted ways out of the two home-screen dead ends
  plus a `[copy]` chip on the focused code block.
- 2026.9.3 — click-first: click-target registry (`Targets`), menus, hover, path row, footer
  chips, fixes to status-row/margin-outline hit bounds, pane keymap ownership, double-click
  slack, hyperlink repaint color and overlay bleed-through, man-page fold glyphs.
- 2026.9.1 — adversarial audit (15 fixes with regression tests), out-of-library links ask
  before opening, the picker opens on the current root, documentation sweep.
- 2026.8.31 / 2026.8.27 / 2026.8.26 — README links made absolute for crates.io, packaging
  guards (`check-packaging.sh` now also checks every recipe's version stamp), `.SRCINFO`
  regeneration, home list rescans every 2 s.
- 2026.8.22 — pager-and-desk slate: diffs as documents, follow mode, code-block cursor/yank,
  continue reading, bookmarks, margin outline, backlinks, frontmatter titles, DEC 2026
  synchronized updates, emoji cell-width workaround (ratatui#2651).

## Open

Packaging / discoverability (quiet channels only; announcements are deferred by maintainer
decision — do not propose them):

1. **nixpkgs by-name PR** — `publish.yml` now computes `carrel-package.nix`'s two hashes
   and builds it after each release, so the file on `main` is submittable from the next
   release on. The PR itself (and a nixpkgs maintainer entry) is still the owner's.
2. **AUR** — recipes stamped and build-tested (`makepkg -f`), blocked on Arch reopening
   account registration (closed since aurweb v6.5.0 during a supply-chain campaign).
3. **Catalog listings** — drafts in the private notes. awesome-ratatui: ready to submit.
   Terminal Trove: needs a still PNG screenshot first. awesome-tuis: not before 2027-02-12
   (six-month minimum age). awesome-cli-apps: archived, skip.
4. **`.deb`** — built by `.github/workflows/deb.yml` after each Release run; first proven
   at the next release (see `contrib/packaging/README.md`).
5. **Publishing after the tag is automated** (`.github/workflows/publish.yml`, added
   2026-10-21): crates.io, the Homebrew tap, nix hashes, the checksum stamp and COPR. It
   needs three repository secrets — `CARGO_REGISTRY_TOKEN`, `HOMEBREW_TAP_DEPLOY_KEY`,
   `COPR_CONFIG` — set once from this machine; the COPR token expires 2027-02-12.

Every feature line on the README roadmap is checked. Known limits, recorded rather than
planned:

- Hyphenation covers ten languages, chosen only by frontmatter `lang:`; Swedish, Russian
  and Catalan patterns are LPPL-only and stay out. Dutch was relicensed to MIT upstream
  in 2021 with no recorded author consent; it is kept, and it is the one to drop if that
  ever matters.
- The footnote box draws display math and wide tables like the page, but has no keyboard
  scrolling (the box owns no keys; the wheel scrolls it).
- Only the first 1 MiB of a file is read for tags; nested tags deeper than heading level 6
  list side by side.
- Home search misses a match spelled only through an entity or across markup.
- `--plain` and `--render` never divide a word, so a long run exceeds the width.
- In help and the outline, `q` is typed into the filter rather than closing the pane (by
  design: letters are the filter; the border says `esc close`).
- The lightbox and the notes pane cover the whole frame, `≡` and the lamp included (by
  design; both have their own buttons).

Upstream: delete `render::declare_wide_cells` when ratatui#2721 (fix for #2651) ships in a
release — it merged 2026-08-24, but ratatui's latest release is still 0.30.2.

Architecture follow-ups from Q29/Q33 (answered 2026-10-19 in the private notes'
`research.md`), not yet scheduled: the TUI state layer is cell-shaped (`App::new` takes
`u16` sizes; `Action` carries cell coordinates; `app.rs` imports `keys`, which imports
crossterm), so rule 6's grep passes while the GUI could not reuse it verbatim. Recommended
order: widen the discipline grep, move the state layer to its own crate, split `Action`
into shared intents and TUI chrome, save positions and bookmarks as `SrcByte` (a
`DocByte` drifts when flattening changes between releases), document the config-key split.
The GTK4 GUI phase is not started and has no date.

## Recent decisions

- 2026-10-19: **the in-workspace dependency is an exact pin** (`carrel-core = "=<ver>"`):
  CalVer's year is the semver major, so a caret accepted every 2026 core and an unlocked
  install of an older carrel failed to compile. `check-packaging.sh` enforces it.
- 2026-10-19: **a menu row prints a key only when the key does the row's job** — "Go to
  the footnote text" shows none, because `%` works from the top of the view.
- 2026-10-19: **a right-click or drag inside the footnote box does nothing**; opening a
  menu would close the box and describe text no longer on screen.
- 2026-10-19: **export writes to `$XDG_STATE_HOME/carrel/exports/`**, never beside the
  document, and copies the path. CRLF copies are LF everywhere, by decision.
- 2026-10-19: **hex colors are not tags** (`#fff`, `#1e1e2e`); `#bad`, `#cafe` are.

- 2026-09-30: **`carrel --latest` is about where you are standing** — the current
  directory or the folder named, never the folder saved for the file list, which is a
  library chosen once. **A footnote mark with no footnote is not a button.** **Search
  results keep `Back`**, like the tags page (reversing the earlier "no history" call).
  **A mouse-up belongs to the press that began it** (see `ARCHITECTURE.md`).
- 2026-09-30: **Review the finished diff cold before pushing.** Three agents with no
  reasoning attached — two reading code, one only driving the binary in a pty — confirmed
  about thirty defects past 1,068 green tests, including a panic and a 27-second freeze.

- 2026-09-30: **A hyphen is decoration, so it is a flag on a row and never a byte.**
  The inline-math rule (content must be in the text) does not apply to it; the bullet's
  rule does. English patterns divide English only, and everything fails toward not
  dividing.
- 2026-09-30: **Tags are a generated document, not a pane** — collapsing, the outline
  and search already exist, and a collapsed tags page *is* a tag browser. It is the
  first generated document `Back` can return to (`Desk`).

- 2026-09-29: **Input cannot wait for resize debounce.** Keep the 40 ms debounce
  for idle resizing, but refresh dimensions before input so invisible actions
  cannot execute. Discard obsolete mouse events and redraw the retained terminal
  buffer even when a shrink/grow burst returns to its starting size. Fullscreen
  `Terminal::resize` performs that invalidation without `Terminal::clear`'s cursor
  position query. Regression coverage uses actual PTYs through both entry routes.

- 2026-09-29: **Small terminals prioritize reading space and reachable controls.**
  The brainstorm considered automatic hyphenation, a minimum-size warning, manual
  compact mode, and automatic adaptation. Chosen: automatic, reversible adaptation,
  full-window compact panes, and scrolling for overflow. A resize must not change
  preferences, document bytes, search matches, or the reading anchor; a clipped pane
  must not accept invisible choices. [Ratatui's layout guidance](https://ratatui.rs/concepts/layout/)
  supports deriving layout from available rectangles; [Unicode UAX #14](https://unicode.org/reports/tr14/)
  distinguishes line breaking from language-dependent hyphenation. The latter remains
  separate work rather than silently assuming English for every document.

- 2026-09-21: **clicks over keybindings where a choice exists.** The beginner
  slate adds bindings only for keys that were already unbound, and reaches for
  a painted, clickable affordance everywhere else. `?` and `Ctrl-F` keep their
  vim/less meanings for this reason. This refines, and does not overturn, the
  2026-09-02 call that keybindings stay as they are: that one was about
  relocating existing commands behind `Alt+`, which is still not happening.
- 2026-09-07: documentation restructured into `AGENTS.md` + `docs/`; `CLAUDE.md` is now just
  `@AGENTS.md`. Note the public/private split: the old `CLAUDE.md` was gitignored because the
  maintainer keeps narrative/strategy material in the private notes repo. These new docs were
  written to be publishable (how-to and code facts) and are tracked alongside the code.
- 2026-09-03: `merman` sibling crates pinned exactly; shipped in v2026.9.23.
- 2026-09-01: `release.yml` hand-edited (gate job, SHA-pinned actions); `dist init` is
  forbidden; recipe version stamps move into the release commit.
- 2026-08-22: clicking a URL copies it, never opens a browser; Windows declined (Q19);
  remote URLs declined (written into the README as a position).
