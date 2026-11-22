# Status

Last updated: 2026-11-21 (second adversarial hunt: eight hunters, three rounds of review)

## Current state

- **Local build-cache changes retained (2026-11-11):** dev/test builds retain line tables
  without full symbols or incremental caches; `./scripts/build.sh [--release]` cleans this
  checkout's `target/` before building when it exceeds 4 GiB (`CARREL_BUILD_CACHE_MIB`
  overrides). Plain Cargo commands do not run cleanup.

- **Latest release: v2026.10.21** (2026-10-21), the first published by
  `.github/workflows/publish.yml`: crates.io (both crates), the Homebrew tap, COPR
  (2026.10.21-1), the `.deb`s, and the checksum stamp with real nix hashes
  (`38f19d0`). Its GitHub Release was created by hand from the run's verified
  artifacts after `release.yml`'s `--target` hit a 403 (fixed on `main`, unproven until
  the next tag). Its nix build fails one test that assumed `/tmp`; fixed on `main`, so
  the nixpkgs PR should wait for the next release.
- **Unreleased work (2026-10-31 through 2026-11-21):** the GUI-preparation work from
  Q29/Q33, three known limits cleared, the reading-input and navigation fixes, and the
  2026-11-14 and 2026-11-21 adversarial hunts' fixes below. The next release is the first with
  three crates, so it is also the first test of `publish.yml` publishing `carrel-state`
  (not yet on crates.io; the name was free on 2026-10-31).
- Previous: v2026.10.8 (2026-10-08) — the crash on closing the terminal fixed, and the
  reader-beside-an-agent slate.
- Previous baseline: all five local gates passed on 2026-11-02: **1,412 tests**, zero-warning clippy,
  formatting, architectural discipline and packaging, plus the 1.95 MSRV check.
  `pty.rs::closing_the_terminal_ends_the_reader_without_an_abort` failed intermittently
  under load: a race that exited 1 (fixed 2026-11-02; 0 in 200 stressed runs after, 4 in
  120 before). The one exit 101 (a panic) seen on 2026-11-02 never reproduced; the test
  now keeps carrel's stderr, so a recurrence will say where. Color-sensitive PTY tests need
  `NO_COLOR` unset in this shell.
- **Decision made 2026-11-02:** `q` over a footnote box closes the box and stops, as it
  closes every other pane (it used to close the box and then the document).
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
  authoritative done/open list. What remains on it is packaging (AUR and nixpkgs)
  and the GUI.

## Recently completed

- Unreleased (2026-11-21) — **a second adversarial hunt: eight hunters, five fixers, then
  four rounds of cold review of the fixes.** Hunters: cargo-fuzz over core with invariant
  oracles, a random-sequence fuzzer over the real key and mouse dispatchers, a black-box pty
  tester, a security hunter, a performance hunter, a documentation-versus-behavior checker
  and two cold code reviewers. About ninety reproduced defects, each fixed test-first with
  the test break-checked; four rounds of review and three more fuzz runs found about
  forty more in the fixes themselves (regressions in every round), fixed the same way.
  Gates: **1,856 tests**, zero-warning clippy on 1.97, fmt, discipline, packaging, `cargo doc -D warnings`, MSRV
  1.95; parse 1.5 ms/MB, height pass 4.1 ms/MB ASCII and 12.8 ms/MB CJK, resize 4.6 ms and
  live reload 14 ms per MB (`cargo bench`, quiet machine).
  - *Crashes.* Math is scanned as pulldown-latex tokenizes it (`math::nesting`): `%`
    comments, Unicode spaces, `\frac+`, `\sqrt[ab]`, `\sideset` and macro definitions
    all reached its recursion or its lexer and aborted; `\def`/`\let`/`\newcommand`,
    `\char` and `\buildrel` show as source, and a non-ASCII space is made plain in the
    copy the parser reads (`plain_spaces`). A one-column table ending in an empty row
    crashed `--plain`, `--render`, a pipe and the columns view (`layout::table_line_ends`).
    A command-line argument that is not UTF-8 panicked (`args_os` throughout;
    `cli::split_target` on bytes). A mermaid sequence diagram of 3,000 participants took
    4 GB: diagrams are bounded before they are drawn (`diagrams.rs` module doc has the
    limits and the measurements). A poisoned index cache panicked at every start. A sparse
    config claiming 100 TB aborted (`state::read_regular`, `MAX_STATE_BYTES`).
  - *Data.* Two readers on one document keep each other's bookmarks and notes: every save
    re-reads its file under `flock` on `<state dir>/.lock` and merges three ways against
    what the session last saw (`state::merge3`, `sync_marks_in`, `marginalia::sync_in`); a
    note both edited is joined once, by paragraph. A kept place is written
    `…^<prev>^<next>` — the 2026-11-14 form had a comma in it and a bookmarks line is split
    on commas, which made a phantom bookmark on the first block; old lines are mended on
    read (`state::mark_fields`). A bookmark that could not be saved says so. Config and
    state are read only from regular files, without blocking (`state::open_regular`), and a
    relative or empty `XDG_*_HOME` is ignored (`state::base_dir`).
  - *Security.* The hyperlink pass prints what the finished frame's cells hold, one span
    per style run, so no bidi or zero-width character the cells drop reaches the terminal
    inside a link; its URL is `ansi::osc8_target`. An unknown option is escaped like a file
    name. A FIFO named `x.md` cannot hold the backlinks worker.
  - *Reader.* An unbound Ctrl or Alt chord does nothing in any keymap (`keys::stray_chord`;
    owner decision, below). `L` works from the file list and for a bare file name, streams
    its answers from the document's folder outward, and stops when closed
    (`BacklinkScan`, `links::Query`). A carried drag keeps the last character it holds. A
    menu that fits stays above the status row (`App::menu_rows`). The notes list closes in
    a tiny window. Back clamps by the document it lands on. A piped document has its
    diagrams drawn. `\sqrt[3]{x}` reads `3√x` (`math::brace_root_indexes`). The settings
    pane shows the real theme; theme names match in any letter case.
  - *Diffs and git log.* Hunks are walked by their `@@` counts, so a removed `-- comment`
    or an added `++i` is kept and counted; a `git format-patch` mail is a commit; a
    message's `<word>` and `![[embed]]` show as written; a bare URL is rebuilt from its own
    source bytes (`as_typed`), never matched against the paragraph.
  - *Performance.* Collapse-all, and a resize while collapsed, are linear in the headings
    (`Document::section_ends`; 25,000 commits: 5.5 s to 14 ms). The info card and links
    pane are linear in the links and never resolve during paint. A document's wikilinks
    list their folder once (`wiki::Resolver`; 16,000 notes: 15 s to 0.12 s). A table costs
    half the memory (`TableRec` is flat). A frontmatter card's tag buttons are found once
    per document (`App::card_tags`).
  - *Command line and documents.* Nothing after `--` is an option, with or without a
    terminal. On a pipe every single-dash flag but `-h` and `-V` is a pager's and is
    ignored. The modes that print a pipe stop at the reader's limit. The man page, `--help`
    and README were corrected in a dozen places, with a drift guard for config keys and
    long flags.
  - *Process.* `ulimit -c 0` does not stop systemd-coredump (`core_pattern` is a pipe; only
    a limit of exactly 1 is skipped): crash repros run under `ulimit -c 1`. The two check
    scripts piped into `grep -q` under `pipefail` and failed at random; they read to the
    end now.

- Unreleased (2026-11-14, afternoon) — **the open items cleared, each through its own
  cold review.** Built on the owner's two decisions (outside images: click to show, per
  image; Ctrl-W/Ctrl-U in every text input):
  - *Outside images:* a click opens the lightbox, which names the file (relative path,
    symlink target, end kept at any width); consent only from its `[show]` or a second
    Enter, keyed to a plan generation, for the session; only the consented block re-plans.
  - *Line editing:* Ctrl-W/Ctrl-U in seven inputs, side effects as Backspace; in the picker
    `/` ends a word; in a note neither crosses a line, and Home/End keep to the line.
  - *Drag autoscroll* (`EdgeClock`), in one `Clocks` with auto-read, which the file list's
    loop never ticked; a buttonless motion ends a drag whose release was lost; a drag
    survives a re-parse, so a streaming pipe can be selected from.
  - *Places:* the reload hold keeps the reader's place and the latest few; a paragraph
    edited where it stands is not held (`Place::edited_in_place`).
  - *Cards:* every rule row has a byte of its own (`layout::rule_byte`); a table's line end
    names its line in the column view.
  - *git log:* escapes are planned and settled against the final parse (`diff::settle`),
    breaking a code span at both ends, a few parses per paragraph (15 s → 26 ms at 300 KB);
    a lone CR cannot open a block; bare URLs with `--` link and keep their hyphens.
  - *Parser:* an empty-label wikilink (`[[x|]]`, a pulldown-cmark 0.13.4 replay bug) is read
    as if escaped (`mask_empty_labels`); a NUL is binary even in valid UTF-8; math symbols
    never count toward the nesting guard.
  Gates: **1,677 tests**, zero-warning clippy on 1.97, fmt, discipline, packaging,
  `cargo doc -D warnings`, MSRV 1.95; parse 1.6 ms/MB, height pass 5.5 ms/MB.

- Unreleased (2026-11-14) — **an adversarial bug hunt: seven hunters, then cold reviews
  of the fixes.** Two fuzzers (cargo-fuzz over core, a random-sequence harness over the
  TUI state machine), a black-box pty tester, a terminal-escape and security hunter, a
  hostile-filesystem hunter and two cold code reviewers found about forty reproduced
  defects; every one fixed test-first, each test break-checked against the old code. Cold
  reviews of the fixes then found six regressions in them, also fixed. Gates: **1,607
  tests**, zero-warning clippy, fmt, discipline, packaging, `cargo doc -D warnings`, MSRV
  1.95; height pass 5.3 ms/MB, parse 2.7 ms/MB (`cargo bench --bench wrap`).
  - *Security.* Printed output (`FILE PATTERN`, `--tasks`, the piped list, every error
    naming a file) sanitized: document text drops controls, names are escaped (`\x1b`,
    `\\`, non-UTF-8 as `\xNN`, bidi overrides) — `plain::clean_text`, `plain::shown`.
    Every clipboard write is a `Clip` (`carrel_state::clip`): no C0 but `\n`/`\t`, no
    C1, CR, DEL, soft hyphen or bidi override; one-line fields carry no newline. A folder
    name can no longer inject config lines (values with a line break, control or edge
    space are refused, not escaped — the format stays literal). Images obey the folder
    rule links obey (`escapes_root`, canonicalized) and are decoded once per path, shrunk
    to what a terminal shows, under a 256 MiB document budget; a FIFO image cannot block.
  - *Data loss.* Config and state files a version cannot read are never replaced; a
    non-UTF-8 line costs only itself; `write_atomic` keeps a symlink; non-UTF-8 paths are
    their own state keys; the entry being saved survives the positions cap; a line with
    more fields than this version writes is preserved.
  - *Places.* A kept place saves both neighbours' hashes (`^<prev>,<next>` on the
    unreleased kept form); `state::pick` chooses among repeated blocks the copy with the
    most like neighbours, nearest count breaking ties, and `None` when the saved copy is
    gone (the source place stands). Live reload carries marks by the same rule. A
    reload that cannot find the place or a mark holds the last good one (`ReloadHold`),
    tried FIRST on the next reload while the reader has not moved — `: > f` then a full
    write comes back exactly; a paragraph rewritten for good is followed by its new text.
  - *Reader.* Footnote box: Enter/`q` only when drawn (`App::peek_drawn`), Enter spends a
    pending count or `g`, a box that fits has its own chips. A reload closes a menu and
    clamps the bookmark list. Art blocks (images, diagrams, display math) on the top row
    restore to themselves. Outline and breadcrumb name the section a gap row is in. `zM`
    lands on the outermost folded heading. Drags that end off the text clamp to the
    nearest text (`App::doc_drag_at`); a double click on a heading word no longer folds;
    an overlay edge splits no wide character.
  - *Input.* Bare URLs copy what was typed (each pairs with the next source URL equal as
    typed or unsmartened). Lossy UTF-8 everywhere with one binary rule
    (`carrel_state::text::decode_document`: a NUL in the first 8000 bytes of invalid
    UTF-8, or over a quarter replaced); a note says how many bytes were replaced. The home
    filter matches the shown relative path. Ctrl/Alt chords never type. `FILE:0` opens
    at the top; a negative width is refused; a lone CR is a line ending for line numbers.
  - *Core.* Verbatim provenance means byte-identical to the source. Math is pre-scanned:
    group depth over 256 or a bare-argument chain over 64 shows as source (pulldown-latex
    was quadratic in depth and overflowed its stack on `\sqrt\sqrt…`).
  - *Performance.* A pipe stops at 64 MB or 2,000,000 lines with a note, re-parsing no
    sooner than 3× its last parse; a 52 MB file opens in about 1.4 s instead of up to
    17 s (one layout on open, tab columns measured only when a tab comes, huge paragraphs
    wrapped per visible chunk); the idle rescan waits 50× its last cost (2 s to 5 min).
  - *Round two* (a second black-box tester, a second fuzz run and two more cold reviews,
    against the merged fixes): images re-plan when the file list's root moves (the
    verdict had stuck both ways — one way bypassed containment); a direct open is rooted
    at the working directory when the file is inside it; saves keep a file's mode and
    refuse a read-only file, new state files are 0600; Ctrl-H erases in every text
    input; a drag may start on a gap row or past a line's end; a wrapped folded heading
    ends its last row with `…`; a file read empty twice keeps its bookmarks; a reload
    never leaves the reader under a fold; a cut pipe keeps its label through Back, and
    the no-tty fallback says it was cut; a `<!--` in a commit message no longer turns a
    later `--` into a dash (keep_hyphens planned on a parse its own escapes changed;
    keep_hyphens, unspan and unbreak were replaced that afternoon by `diff::settle`).
  - *Round three* (a cold review of round two): the diff plan's stand-in for `<!--` is
    a letter, since a space made `*<!--` a list item (stand-in since removed with
    `keep_hyphens`); `write_atomic` refuses a target
    that is not a regular file rather than block opening a FIFO; a refused chmod no
    longer fails the save (vfat — checked with an `fchmod` shim only, no test); a
    document reopened under the same root shows its images again (pty-checked, with a
    control); a jitter on a gap row selects nothing.
  - *Process.* CI's clippy is pinned to 1.97 and this machine's default is 1.98: three
    pushes passed locally and failed CI on `duration_suboptimal_units`. Run clippy as
    `cargo +1.97 clippy` (DEVELOPMENT.md).

- Unreleased (2026-11-11) — **small reading improvements, with no new dependencies or
  settings.** All five local gates passed: **1,472 tests passed, zero failed, two
  intentionally ignored**, zero-warning Clippy on Rust 1.97.1 and 1.99.1, formatting,
  discipline and packaging. All targets also check on Rust 1.95.0 (MSRV) and 1.97.1.
  Extended runs passed 5,000 generated documents, 2,000 mixed navigation sequences,
  20,000 hostile commit messages, and 50,000 partial rewrites (77,000 cases total).
  The release build and five optimized terminal smoke checks passed. Implementation
  commit `0477112` passed CI, including the MSRV job. A 15-minute idle-pipe soak used
  3.48 seconds of CPU with sampled RSS unchanged at 7,004 KiB, and quit cleanly with
  the writer still open (one small-document Linux measurement, not a resource bound).
  - *Paste into search, help and outline* once per paste; bracketed paste stays on throughout
    the TUI so pasted newlines and `q` cannot become commands. Inputs preserve whole graphemes
    at the 256-scalar paste limit and on Backspace; tabs separate words.
  - *Recent searches* — twenty accepted queries, Up/Down or clickable hints, draft restoration,
    memory only. Home retains them through direct-file, tutorial and pipe entry routes.
    Backward search displays `?` and reports wrapping to the end.
  - *Paths keep their literal meaning* before shell unescaping; joined shell quotes and quoted
    local file URLs decode without evaluation. Existing spaces, backslashes, quotes and broken
    symlinks win over a different decoded filename. Pasted line/heading references also work
    with spaces in the filename and with extensionless files.
  - *Live edits and Back/Forward keep the paragraph.* Kept places carry the viewport,
    search Escape destination, selected match and history destinations through insertions.
    Shared reparse logic retains named folds and bookmarks for files and streams.
  - *Keep up with watched files* via `F` or the menu. Navigation/selection stops motion;
    auto-read and keeping up are exclusive. Background pipe EOF affects only the pipe.
  - *Retained pages* — tutorial, search results and tags keep notes, bookmarks and collapsed
    sections for Back/Forward. Pipe state stays separate. Home opens interactively from
    `--tutorial` and piped input instead of exiting or printing a plain list.
  - *Empty input and terminal sizes* no longer assume a block or screen cell exists. A pipe
    awaiting text says so. Invalid UTF-8 is decoded iteratively instead of recursively;
    producer queues and event-loop batches are bounded, with short burst coalescing.
  - *Git message rendering* now uses the Markdown parser's actual code-span/autolink ranges.
    A full-suite property run found a numbered continuation beside unmatched backticks
    turning command hyphens into a dash; the seed and a named regression test are retained.
  - *Preview and tooling* — reproducible static README screenshot; cache helper preserves
    Cargo's cleanup marker guard, confines output to the checkout, and has isolated tests.
    Rust 1.99 Clippy assertions fixed without changing CI pins or suppressing lints.
    A stricter API-doc build found broken/private links; these are corrected and checked in CI.
    A mutation-checked property also covers search and bookmark validity during partial rewrites.


- Unreleased (2026-11-02) — **every known defect fixed**, each test-first and each test
  break-checked against the old behavior, then cold-reviewed by an agent that found two
  regressions (both fixed, below). Left as is: a bare URL in image alt text becomes a
  link, as a `www.` one always has.
  - *Bare `http(s)://` URLs link* (`document.rs::scheme_autolinks`, beside the `www.`
    scanner, both trimmed by `autolink_end`). The URL copied is the SOURCE's where smart
    punctuation rewrote the text (`it's`, `--`), paired by order with a scan of the
    source; not where an entity or escape is in it. A URL whose text event runs straight
    into markup (`\_`, an emphasis opening inside it, a footnote mark) reaches the scanner
    cut short and stays text (`runs_on`, peeked in `run`) — a link to the wrong page is
    worse than none. Frontmatter values link too.
  - *`q` over a footnote box* is `Dismiss` in `key_action`, over the reader's map only.
  - *Navigation lists* (`paint_navigation_entries`, shared by bookmarks and the outline):
    the selected entry grows only into rows the others do not need, and a cut label
    continues on its last row instead of restarting (taken from the label, so a token
    a row cut is not given a space). A list longer than the pane scrolls anyway, so there
    the selection may take up to half of it. The outline had the same bug.
  - *The footnote box* is placed on `App::peek_page()` (text top to footer) by
    `Peek::place`: below its mark, else above, else the roomier side, sized to it; never
    over the mark. `view`, `scroll`, `scrolls` and `zone` all take that page, so keys,
    paint and hit-test agree.
  - *Continue-reading* is re-read with the file's own row on every return home
    (`main.rs::came_home`).
  - *Gap rows anchor at the separator byte after their block* (`view::row_anchor`,
    `view::row_of`), which `block_at_doc` hands to the next block and `row_of` turns into
    the row just above it; it survives a resume through `Place` with no format change.
    Property `every_top_row_restores_to_itself` over the conformance corpus at four widths;
    found on the way, an empty first text row (math shown as source) restored a row late
    (`visual_row_of`). The cold review found `za` from a gap row left the folded heading a
    row above the screen: `after_fold_change` now asks about the block ABOVE a separator
    anchor (guard in `tests/folds.rs`). The one bounded exception: the gap under an EMPTY block (a `---`
    rule) shares that block's only byte and comes back one row up.
  - *A forced cut that touches spaces cuts at them* (`units::cut_at_spaces`), unmarked,
    the spaces carried as the head's trailing whitespace; `pack` ends the row at the head's
    content end.
  - *Home search's math-view cache* is pruned to the file list each time a search finishes
    uncancelled.
  - *Places keep their block by content* (`state::Place::kept`, `@<fnv64>.<nth><±off>` on
    the unreleased `s` form): positions and bookmarks resolve to the same text after an
    edit made while carrel was closed; the source place stands only when the block's text
    is gone. Guard: `tests/places.rs::a_kept_place_follows_its_text_down_past_an_insertion`
    (every byte of a varied document, with a paragraph inserted above).
  - *The intermittent PTY failure* was the main loop hitting `EIO` on the closed terminal
    before the watchdog: exit 1, `carrel: Input/output error`, about 1 in 30 closes under
    load (reproduced 4 in 120). `TerminalGuard::drop` ends that path as the hang-up it is
    (129), never during a panic. The test now keeps carrel's stderr.

- Unreleased (2026-10-31) — **ready for a second frontend, and three limits cleared.**
  - *A third crate, `carrel-state`*, for what every frontend shares above the model:
    `Intent` (109 document-addressed intents; the terminal's 24 chrome actions stay in its
    `Action`, converted by exhaustive matches both ways), and persistence and file
    discovery (`config`, `state`, `scan`, `grep`, `tags`, `links`, `wiki`, `fuzzy`,
    `marginalia`, re-exported by `carrel` under their old names). Published between core
    and carrel; every in-workspace pin is exact and checked.
  - *Rule 6 checks the closure.* `app`, `footer` and `menu` had reached crossterm through
    `keys` since the first release while the direct-import grep passed. What the keys are
    called is data in `keymap.rs` now (the help tables carry the action a clicked row runs,
    held to the dispatcher by a test); rule 6 fails on any terminal crate named in a listed
    module or any `crate::` edge off the list; rule 7 holds `carrel-state` to no UI
    dependency and no terminal geometry. Every check break-checked.
  - *Positions and bookmarks are source places* (`state::Place`, `s1024+16`): exact under
    the core that saved them (file unchanged), in the right run under another; the old form
    still reads, and a line or mark this version cannot read is written back unchanged
    (2026.10.21 and earlier discard them on their next save). A diff's place is in the
    markdown carrel generates from it, so only the first guarantee holds there. A reload
    carries bookmarks by block content.
    Found on the way, a shipped bug: `open_path` loaded bookmarks before setting `file`, so
    a reopened document had none and the next one opened showed (and on `m` saved) the
    previous one's.
  - *The config-key split* is written in `config.rs`: shared keys bare, terminal keys keep
    their names, GUI keys get a `gtk.` prefix.
  - *Footnote box keys*: the page's scroll keys scroll the box exactly while it shows there
    is more, and stop at its ends.
  - *`--plain` and `--render` keep the width* (`carrel_core::Division`): no word divided,
    nothing added, and only a run wider than the row runs past it, on a line of its own;
    styled `--render` cuts such a run as the screen does, with a faint mark.
  - *Home search finds what the reader finds* (`carrel_core::SourceFilter`): entities,
    markup, list and table structure, and rendered inline math; four workers, and faster
    than before on every query measured.

- 2026.10.21 (released 2026-10-21) — **publishing after the tag moved into CI, and the
  release that proved it.** `.github/workflows/publish.yml` runs when `Release` succeeds:
  `cargo publish` (core, then carrel) with a crate-page link check and an unlocked
  `cargo install`; the formula committed to the tap with a write deploy key; the nix
  hashes found on a hosted runner and built with; `scripts/stamp-release.sh` committing
  the checksum stamp to `main`; the COPR build from the tag, waited on. Every job skips
  work already done. A dispatched trial against v2026.10.8 skipped everything as it
  should. Two defects surfaced on the way: CI's property test found `$$` in a commit
  message forming math that hid a code span, so `--` printed as a dash (`$` is now
  literal in messages); and a render test that hard-coded `/tmp` failed in the nix
  sandbox (`TMPDIR=/build`), fixed on `main` after the tag. The Release run's host job
  then failed with a 403 because `--target` named a commit that was no longer `main`'s
  head; the release was created by hand from the run's verified artifacts, `deb.yml` and
  `publish.yml` dispatched with the tag, and `release.yml` now uses `--verify-tag`.

- 2026.10.21 (implemented 2026-10-19/20) — **the open list, cleared, then three cold
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
2. **AUR** — recipes prepared but not published. The AUR RPC returned no `carrel` or
   `carrel-bin` package on 2026-11-11. Account registration could not be verified through
   its browser challenge; do not treat the old registration-closure claim as current fact.
3. **Catalog listings** — drafts in the private notes. awesome-ratatui: ready to submit.
   Terminal Trove: still PNG and reproducible capture are now ready (`assets/screenshot.png`).
   awesome-tuis: not before 2027-02-12
   (six-month minimum age). awesome-cli-apps: archived, skip.
4. **Prove the `release.yml` `--verify-tag` fix** at the next release. Until it has run
   once, do not push to `main` between a tag and its Release run's host job.
5. **Before the next automated publication** (`.github/workflows/publish.yml`): the
   existing repository secrets are `CARGO_REGISTRY_TOKEN`, `HOMEBREW_TAP_DEPLOY_KEY` and
   `COPR_CONFIG`; the COPR token expires 2027-02-12.
   **Before the next tag**, `CARGO_REGISTRY_TOKEN` must be allowed to publish a *new*
   crate (`carrel-state`); a token scoped to the two existing crates publishes core and
   then fails part-way through the release. Checked 2026-11-02: the secret dates from
   2026-10-21, before `carrel-state` existed; crates.io shows a token's scopes only on its
   website (the API refuses to list them); the name `carrel-state` is still free.

Every feature line on the README roadmap is checked. Known limits, recorded rather than
planned:

- Hyphenation covers ten languages, chosen only by frontmatter `lang:`; Swedish, Russian
  and Catalan patterns are LPPL-only and stay out. Dutch was relicensed to MIT upstream
  in 2021 with no recorded author consent; it is kept, and it is the one to drop if that
  ever matters.
- Only the first 1 MiB of a file is read for tags; nested tags deeper than heading level 6
  list side by side.
- `--plain` and `--render` keep the width except for one run wider than the whole row;
  aligned tables in them can still exceed it, and a URL may wrap after a `/` (nothing is
  added, but it lands on two lines). Code lines are never cut in either, styled or not.
- Home search keeps each file's inline-math view until the file's mtime or length changes.
- In help and the outline, `q` is typed into the filter rather than closing the pane (by
  design: letters are the filter; the border says `esc close`).
- The lightbox and the notes pane cover the whole frame, `≡` and the lamp included (by
  design; both have their own buttons).
- In a single-column table shown as cards, a row beside an EMPTY cell can come back one
  row off after a resize, when that exact row was on top: the rule under an empty body
  cell (one lower), the top rule over an empty header cell (as the first body row), and
  the blank text row of an empty last cell (as the rule under it). The blank cell's row
  holds its line's only byte, so two rows share it. Every other rule is named by the
  `\n` ending the card above it (`layout::card_rows`, guarded by
  `view::tests::every_card_row_restores_to_itself`).
- Math with more than 64 commands waiting on an argument at once, written without braces
  (`\sqrt\sqrt…x`), shows as source: pulldown-latex parses a bare argument by recursion.
  A long ROW of them (`\hat a\hat a …`) renders at any length. More than 64 unknown
  commands in one group, or 64 comments in a row, cost a formula its rendering too
  (`math::MAX_BARE_CHAIN`, `MAX_COMMENT_RUN`).
- A mermaid diagram past its cost limits (2,000 items; for a flowchart 400 nodes, or
  nodes × edges over 160,000; for a class or ER diagram nodes² × relations over
  20 million, a tree excepted; 64 KiB of source outside comments; a document's diagrams
  together 5 s of drawing) stays source, with a caption. The worst shape that still
  draws takes about a second, off the UI thread.
- Display math (`$$…$$`) inside a table cell is not a cell's content: the row splits
  over three lines and leaves an empty math block above the table. Inline math in a
  cell is fine.
- Only a document's first 2,000 links are judged dead or alive; past that a dead link
  looks like a live one (`App::find_dead_links`: each judgment is a `stat`).
- In a note draft, Up and Down are unbound: the box wraps at paint time
  (`annotation_render::draft_rows`) and the editor does not know its width, so moving by
  stored line would skip wrapped rows. Home and End go to the cursor's line.
- Files open synchronously (about 1.5 s at 52 MB); a model node costs ~230 bytes, so a
  64 MB file of one-line headings still costs gigabytes.

Upstream: pulldown-cmark 0.13.4 repeats the rest of a paragraph after an empty-label
wikilink (`[[x|]] abc` read `]] abc abc`); `document::body_events` reads such a link as
if escaped (`mask_empty_labels` masks its brackets with same-length `%`, parses that, and
re-borrows every string from the real source). 0.13.6 fixes it upstream and is what the
lockfile holds since 2026-11-21 (one of 37,545 markdown files on this machine parses
differently under it, a list's end, and more correctly); the mask stays while the
declared floor is 0.13.4, and reads `[[x|]]` the same under both.

Upstream: pulldown-latex 0.8.0 emits a root's index after its radicand with nothing
around it (`\sqrt[n+1]{x}` is the radicand group, then `n`, `+`, `1`), so the index is
braced before parsing (`math::brace_root_indexes`).

Upstream: delete `render::declare_wide_cells` when ratatui#2721 (fix for #2651) ships in a
release — it merged 2026-08-24, but ratatui's latest release is still 0.30.2.

For the owner (2026-11-21, not changed):
- *Esc then a key on a slow link.* With unbound chords inert, Esc-then-`j` arriving in one
  read is Alt-j and both keys are lost (before, the `j` acted and the Esc was lost). One
  way out that keeps the decision: read Alt-letter as Esc alone in the maps that type
  nothing. Not done, because "does nothing" was the decision.
- *British spellings in user-facing text* that predate this work: `--help` (`colours`,
  `honoured`), the man page (11), the README (15), the fish and zsh completions. Not
  swept, by the standing rule against sweeping spellings nobody asked to change.
- *Exported notes are the note's text as typed*, so read back as markdown a `<tag>` in a
  note vanishes and its lines join. Escaping them would change what `C` copies.
- A mouse report with a zero coordinate panics inside crossterm 0.29 in a debug build
  (an overflow check; release wraps). No conforming terminal sends one.

Known defects, all low, found by the last fuzz run of 2026-11-21 and left: a
`git format-patch` mail with two `Subject:` headers shows only the last, and header lines
with no blank line after them flow into the author's paragraph (malformed mail only);
`Document::copy_text` of a range cut inside a kept flag returns loose tag characters (no
input makes such a range: a flag is one cluster); hidden characters inside `H~sub~O`
count toward the 16-byte limit and can switch the script off; `[[[x](|]]'` loses five
characters (an empty-label wikilink whose target holds `](`, before a `'`); `git log
--stat` flows its diffstat into a paragraph. Otherwise none open. The last, stray backslashes in `git log | carrel`, is fixed
by planning every escape the message-to-markdown pass adds as data and settling it
against the parse of the final text (`diff::settle`), instead of layering string passes
that each planned on a parse the next one changed; `only_the_headings_it_wrote` now
asserts no backslash the author did not write. Settling breaks every offending construct
a parse finds at once (a code span by both its backtick runs) and escapes every opener
after 16 rounds, so a paragraph settles in a few parses. Still open, accepted: an email
autolink with `--` or `$` in its local part (`<a--b@c.d>`) shows as text, not a link.

Everything else the 2026-11-14 hunt reproduced has a fix and a break-checked regression
test, apart from the limits listed above; see Recently completed.

The Q29/Q33 architecture follow-ups are done (see Recently completed). What a GTK frontend
would still find: the TUI's `App` is sized in cells and stays in `carrel`, deliberately — a
GUI shares `carrel-core`, `carrel-state` and the `Intent` vocabulary, and writes its own
state machine over them, dispatching on `Intent` with one explicit arm for what it does not
implement. The GTK4 GUI phase is not started and has no date.

## Recent decisions

- 2026-11-21: **an unbound Ctrl or Alt chord does nothing** (owner). Each keymap names the
  letters it binds with Ctrl (`keys::stray_chord`); Alt is never bound. Ctrl-J/K/N/P and
  Ctrl-Space no longer act on the reader's page, where they were never bound. The cost:
  a terminal reads Esc and a letter arriving together as Alt-letter, so on a slow link
  Esc-then-`j` typed fast now loses both keys where it used to lose only the Esc.
- 2026-11-21: **on a pipe, a single-dash flag is a pager's and is ignored** (all but `-h`
  and `-V`): every letter is a `less` option, and a pager that refuses one shows nothing
  of what was piped to it. An unknown `--long` option is an error everywhere.
- 2026-11-21: **two readers' edits to one note are joined, by paragraph, once**; nothing
  either wrote is dropped. A reading position stays last-writer-wins.
- 2026-11-21: **what links here searches the folder being read in**, the document's own
  folder first, and says `first 200 found` when it stopped at its cap.
- 2026-11-21: **a formula that defines a macro shows as source** (`\def`, `\let`,
  `\newcommand`; also `\char`, `\buildrel`): expansion can write a chain no scan of the
  source sees, and the parser's recursion aborts the reader.
- 2026-11-21: **printed output leaves out what the cells refuse**: a Hangul filler or a
  lone medial or final jamo takes no cell on screen and is not printed by `--plain` or
  `--render` either. Tag characters and the interlinear annotation controls never enter
  the display text (the parser's `Builder::push` leaves them out, provenance kept), so
  search, selection, copy and printing read what is shown; names and titles are
  stripped where they leave (`carrel_core::hidden::strip`). Only the flags of England,
  Scotland and Wales are kept. Bidi marks stay in printed body text, which may be
  right-to-left.
- 2026-11-21: **a hyperlink is an address a terminal can open, or is not sent**: a
  destination with a scheme as written; a local file that resolves, or a placed wikilink,
  as an absolute `file:///` URI; none for `#fragment` alone, a dead link, a path outside
  the folder, or anything on a pipe. The link selected with Tab is a hyperlink too.
  `--render` names a local file only when its output is a terminal: redirected, it
  would put the reader's home path in a file people share.
- 2026-11-21: **a wikilink that leaves the folder is refused before the disk is asked**
  (`wiki::Bounds`), as a markdown link always was: a document is untrusted, and a `stat`
  of wherever it points is not its to order. With no library root the folder is the
  document's own. Such a link is not known to be dead; following it asks first and
  then opens it, as a markdown link out of the folder does.
- 2026-11-21: **a `git format-patch` mail reads as `git log -p` shows its commit**:
  `Subject:` is the title, `From:` the author, the diffstat a plain fence. A hunk keeps
  the sigil-led lines its header under-counted.
- 2026-11-21: **a diagram is refused by what it would cost, not by how big it is**, and
  says so on its gap row (`too large to draw — shown as text`). The limits and the
  measurements behind them are in `diagrams.rs`'s module doc.

- 2026-11-14: **a drag across a collapsed section copies the hidden text too**, as
  editors with folding do (the selection is a byte range; a fold hides rows, not text).

- 2026-11-14: **an image outside the folder is shown when the reader asks, per image,
  for the session.** Its alt text is a `click to show` button (also a right-click row)
  that opens the lightbox, which names the path at every width it draws at (wrapped,
  or cut from the left so the file name stays; relative to the root when shorter;
  `link.png → target` for a symlink); only its `[show]` or a second Enter consents —
  never a press on the block, whose alt text the document chooses. Consent
  is the image's canonical path, kept in memory only (`App::image_consent`), so a
  restart asks again, and an answer carries the plan it was asked under
  (`App::image_plan`), so a stale one consents to nothing. It survives a reload and a
  switch to another document naming the same file: consent is to the file, and nothing
  a second document can do with it leaves the screen. Consent decodes that block alone
  (`images::plan_consented`) from what the budget has left.
- 2026-11-14: **a file named on the command line is rooted at the working directory
  when it lies inside it**, else at its own folder — so `carrel docs/guide.md` shows
  `../assets/x.png` and follows in-repo links without asking; the README's "the folder
  you are standing in". Images outside the root are never opened.
- 2026-11-14: **config values that cannot round-trip are refused, not escaped**; the
  file stays bare `key = value`, so every hand-written config reads as before.
- 2026-11-14: a short footnote box shows `footnote` chips (`enter go to it`, `esc
  close`) instead of the reading row — the reading row's chips would not do what the keys
  do while it is open.

- 2026-11-11: **Improve existing reading gestures before adding another pane.** Search recall,
  safe paste, stable return positions and Home continuity fit the novice workflow without
  dependencies, persistent search history, or additional configuration.
- 2026-11-11: **A background producer is not the displayed document.** Pipe arrival/EOF,
  watched-file keeping up, and pathless page state use explicit identities.
- 2026-11-11: **Use the Markdown parser for Markdown boundaries.** A line that resembles
  an ordered list item does not necessarily interrupt a paragraph or its code span.

- 2026-10-31: **the shared layer is `carrel-state`, and `App` is not in it.** `App` is
  shaped by cells; moving it would have repeated Helix's mistake one crate over. What
  moved is what carries no geometry: intents, persistence, discovery.
- 2026-10-31: **a saved place is a source byte plus a correction**, not a source byte:
  provenance is per run, so a source byte alone resumed a rewritten paragraph at its top.
- 2026-10-31: **a footnote box borrows the page's scroll keys only while it says there is
  more**, and stops at its ends; a box that fits takes none.
- 2026-10-31: **linear output divides no word to fill a row**, and a run wider than the
  row stands alone; only styled `--render` cuts it, faintly marked.

- 2026-10-21: **everything after the tag is CI's** (`publish.yml`). By hand: docs sweep,
  release commit, tag, watching runs by SHA. Secrets `CARGO_REGISTRY_TOKEN`,
  `HOMEBREW_TAP_DEPLOY_KEY` (a deploy key scoped to the tap, not a PAT), `COPR_CONFIG`.
  crates.io is now published after the gated build, not before the tag.
- 2026-10-21: **a `$` in a commit message is literal.** It is shell (`$$`, `$HOME`), and
  math outranks a code span, which let message text escape the hyphen guard.

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
