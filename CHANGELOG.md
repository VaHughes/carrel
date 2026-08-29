# Changelog

Versions are calendar dates, `YYYY.M.D` (Eastern time).

## Unreleased

- **A file you just wrote appears in the list without a restart.** The home screen walked
  the tree once, at startup, so a document created while it was up stayed invisible until
  you quit and reopened. It now takes another quiet look every couple of seconds: a new
  file arrives at the top of the list, a deleted one leaves it, and a file you have just
  edited moves up to where its new modified time puts it. Nothing blinks while it happens —
  no `⟳ scanning…`, no jumped selection — and a walk that finds nothing changed costs
  nothing, so the list stays current without the index cache being rewritten on a timer.

## 2026.8.27

- **The logo and the demo render on the crate page.** They had been broken on
  <https://crates.io/crates/carrel> since launch while rendering correctly on GitHub, and
  three ordinary links were broken beside them. crates.io resolves a relative link in a
  readme against the crate's own directory in the repository, not the repository root —
  and carrel's readme lives at the root and is inherited by both crates, so every relative
  path was served one directory too deep. They are absolute URLs now, which is the only
  form that is correct in both places. `check-packaging.sh` fails on a relative one.

- **The AUR recipe would have failed its integrity check.** `.SRCINFO` is a flattened copy
  of the `PKGBUILD`, so a hand-edited version number left the source line fetching the
  v2026.8.21 tarball while the checksum beside it was v2026.8.26's. It has never been
  installable from the AUR — registration there is still closed — but it is correct now,
  and `check-packaging.sh` fails if the two files disagree again. The Fedora spec's
  changelog also regained the two releases it had skipped.

## 2026.8.26

- **A `<details>` block folds like a section.** The summary line wears the same markers a
  folded heading does — a dim `▸ …` folded, a `▾` open — and folds the same way: `za` at
  the top of the view, click the summary, `zM` to collapse every one of them at once.
  Search always unfolds its way in, exactly as it does for sections. Nested details fold
  independently; a `<details>` with no visible summary text offers no fold at all, because
  a fold with nothing to keep visible is not a fold. The regions are recorded in the core's
  document model as byte ranges, so they survive a resize by the same construction that
  keeps search hits and bookmarks alive.

- **`%` jumps between a footnote reference and its definition.** Under a reference, `%`
  takes you to that reference's definition; inside one, back to its first reference; above
  everything, to the first pair in the document. Either way a history entry is pushed, so
  `Ctrl-O` returns — a jump is a link follow in spirit. References inside code samples are
  prose about footnotes, not footnotes, and are skipped.

- **`l` asks what this note points at** — the mirror of backlinks `L`. Every distinct
  destination in the document, in reading order: wikilinks resolved the way the reader
  resolves them, relative links against the file's own directory, external URLs shown but
  never opened. Enter follows a local target through the ordinary history machinery;
  pressing it on an external link says so instead of fetching.

- **`"` lists your bookmarks.** `'` walks them blind; this shows every mark in the document
  with its context line, pre-selected at or after where you are reading, Enter jumps,
  Ctrl-O comes back. Rows derive from the live list each frame, so clearing a mark under
  the open pane is reflected before your next keystroke.

- **The pickers fuzzy-match now, ranked best-first.** The home screen's name filter and the
  outline picker both took exact substrings; both now take an in-order subsequence and rank
  by alignment — matches at word boundaries (`/`, `_`, `-`, `.`, camel humps) beat word
  interiors, tight runs beat scattered ones, and the *best* alignment wins rather than the
  first: `xaabbx` queried for `ab` finds the adjacent pair, skipping the leftmost `a` that
  strands the rest. A hand-rolled dynamic program, two rows of cells, no dependency — a
  folder of 100,000 notes still refilters on every keystroke. Ties keep the mtime order
  they had.

- **`I` shows the document card**: words, total reading time, headings, code blocks,
  tables, images, math, links local and external, tasks done-of-total, footnote counts,
  bookmarks, when the file last changed. Derived fresh every frame it shows, so nothing on
  it can go stale. (`g` was considered and declined — it owns the `gg` prefix.)

- **`S` spotlights the paragraph.** Everything outside the block nearest the centre of the
  view dims; the measure stays put, positions stay put, only the paint changes. Purely
  presentation — layout never hears about it.

- **`A` reads to you.** Auto-read drifts the view down one row every 300 ms — two hundred
  rows a minute, brisk enough to feel like reading. Any deliberate scroll takes the wheel
  back immediately, `gg`/`G` included, and reaching the end stops it gently with a note.
  The heartbeat lives in the event loop; state stays pure.

- **Task lists are navigable and reportable, still never editable.** `X` jumps to the next
  GFM task item, wrapping, count-multiplied, saying which and whether it is ticked.
  `carrel --tasks FILE` prints every task as checkbox lines and exits — the file untouched.
  The info card carries the done-of-total count. Home-screen progress glyphs were tried on
  paper and left out: counting honestly means reading whole files, and a number read from
  the first 2 KB would be a lie with a font on it.

- **`carrel --render` styles a pipe.** The document as linear text with weight, slant,
  strike and real OSC 8 hyperlinks — for embedding carrel's rendering in another tool's
  output, the thing agents and build logs want. Never an SGR colour: a non-interactive pipe
  has no palette, and guessing one would fight whatever theme surrounds it. `NO_COLOR`
  reduces it to `--plain` exactly. Widths work like `--plain`, stdin included.

- **Places: the directories you keep coming back to are remembered.** Choosing a root in
  the directory picker now records it as a `place = …` line in the config, newest first,
  duplicates collapsing onto their newest visit, capped at eight so it stays a short menu
  rather than a history. The picker offers them ahead of the filesystem's own completions
  whenever the typed path is empty.

## 2026.8.22

- **Scrolling fast no longer eats or strands characters.** A line containing an emoji
  (`⚠️`, `✅` — anything written with a variation selector) could paint as `automatd` instead
  of `automated`, and drop stray letters elsewhere on the screen. ratatui's cell diff emits
  the column *underneath* such an emoji immediately after the emoji itself, and the crossterm
  backend only repositions the cursor when the next cell is not the previous one plus one —
  so in every terminal that gives the emoji its two columns (ghostty and foot both do) that
  write landed a column late, and every write after it in the same run followed, each
  overwriting its right-hand neighbour until the next reposition. It took a fast scroll to
  see because that is when whole rows change at once and the runs get long. carrel now
  declares the true width of those cells, which makes the diff skip the covered column and
  restores the reposition.

- **Every frame is sent as one synchronized update** (DEC mode 2026), so a terminal draws it
  whole or not at all instead of being free to show a half-applied one. Terminals that do not
  know the mode ignore it.

- **The man page documents every reader key again**, including how to page, follow a link,
  and go back — basics it had quietly never carried — and a test now fails the build if a
  key reaches the help overlay without reaching the man page.

- **The home list can show titles instead of file names** (`titles = true`): `title:` from
  frontmatter, else the document's first heading, else the file name as before. Only the
  rows actually on screen are read, and only their first 2 KB, so a folder of 100,000 notes
  costs the same as a folder of ten.
- **What links here** (`L`). Reading a note and want to know which other notes point at it?
  `L` lists them, with the line each link sits on; enter opens one. It understands both
  `[[wikilinks]]` and ordinary markdown links, resolves them the way the reader does, and a
  document that merely says the word is not a link. There is no index to go stale — it is a
  question asked when you ask it.
- **A stated position on remote documents: carrel will not fetch them.** Other readers will
  open a URL you hand them; carrel does not, and now says so in the README as a decision
  rather than leaving it as an absence. It depends on no HTTP client and no TLS library, so
  "it never fetches anything" is something you can check rather than something you have to
  take on trust. Piping covers the real need — `gh pr view 128 | carrel` — and keeps the
  fetch, and the credentials it uses, where you can see them.
- **The outline in the margin** (`outline_margin = true`). On a wide terminal the 90-column
  measure leaves empty space; the section tree can live there instead — every heading,
  indented by level, the ones you are inside lit, click any of them to jump. It folds away
  on a terminal without the columns to spare rather than squeezing the measure, and it is
  off unless you ask, so nothing moves on upgrade.
- **Continue reading.** Carrel has always remembered where you were in every document and
  never mentioned it. The home screen now opens with what you are part-way through — the
  file, how far in, and roughly how long is left — and `1`, `2`, `3` or a click picks one
  up. Documents you have finished, or opened and not read, are not offered: the band is an
  answer, not a history. Nothing appears until you have something to continue, so a first
  run looks exactly as it did.
- **Bookmarks.** `m` marks the place you are reading; `'` walks between marks, wrapping;
  a dot in the margin shows which blocks are marked. They are remembered between sessions
  and survive a resize — they are document positions, not screen positions — though not an
  edit to the file. **`m` used to toggle rendered diagrams and math; that moves to `r`**,
  which was always the better mnemonic for it.
- **`git show | carrel` reads like a document.** A pipe, or a `.diff`/`.patch` file, is
  recognised as a diff and laid out as one: a heading per commit and per file with its
  `+/−` counts, hunks as code, additions and removals coloured from your own theme's
  palette rather than a generic red and green. Because files become *sections*, everything
  carrel already does works on a diff — fold a file with `za`, collapse the changeset with
  `zM`, jump between files from the outline, search across all of it without the matches
  moving when you resize. `git config core.pager carrel` puts it in front of every git
  command that pages, and unknown pager flags are ignored rather than refused.
  A `.md` file is never read as a diff whatever it contains; `--diff` and `--no-diff`
  override the guess.

## 2026.8.21

- **Follow a document that is still being written.** `F` pins the view to the end of a
  growing pipe — an agent writing as it thinks, a build log, a `tail -f` — and any
  deliberate move away detaches it again. It starts **off**, even for a pipe: nothing moves
  under you unless you ask, and `G` on a still-growing document is the other way to ask.
  The footer says `following` while it does.
- **Copy a code block without touching the mouse.** `]` and `[` step between code blocks —
  a bar marks the one in focus — and `y` copies it, fence excluded. The most common thing
  anyone does with an answer from an agent is take the command out of it; that is now two
  keys rather than a careful drag.
- **Windows is not coming, and the roadmap says so now.** The port was scoped and costed
  months ago and has been listed as planned ever since. It is declined: there is no Windows
  machine on this project, so a Windows build could only ever be verified by CI, never by
  someone reading in a real terminal. Carrel is a Unix program — six targets, and that is
  the set.
- **`x^2^` and `H~2~O` now render as superscript and subscript.** The markdown parser only
  recognises `^…^` and `~…~` when they follow a space, so the spellings people actually
  write — attached to the word — stayed on screen as literal carets and tildes. Carrel now
  handles those itself, and the result is indistinguishable from the spaced form: the
  markers are consumed, the content is a script run, and search and selection see the same
  text you do. Deliberately narrow, because `~` and `^` are ordinary characters: the
  content must be short and unbroken, `~~strikethrough~~` is untouched, a `~/path` after a
  space is untouched, and URLs are skipped whole.
- **A URL inside a fenced code block is no longer turned into a link.** Bare `www.` addresses
  were being linkified everywhere, code samples included, where they came out styled as
  links and clickable in terminals that support it.
- **The directory picker is an input now.** `d` opens a box you type a path into, and it
  completes against the filesystem as you go — `~` expands, a trailing `/` lists a
  directory whole, a bare word completes against the one you are standing in, and the
  matches redraw on every keystroke. The old fixed menu of `~/Documents` and
  `~/Documents/GitHub` is gone: those were a guess, and they found nothing at all on a
  machine that keeps its files anywhere else. With nothing typed the picker offers the
  current directory and the top level of your home. `$HOME` itself is still never offered
  — scanning all of it descends into every cache and container on the machine — but you
  can type it. `Ctrl-J`/`Ctrl-K` (or `Ctrl-N`/`Ctrl-P`, or the arrows) move; `Esc` clears
  the path, then closes. The box grows downward from a fixed top edge, so the input row
  holds still while the match list changes under it.
- **`Ctrl-J` / `Ctrl-K` move anywhere typing owns the letters** — the directory picker, the
  name filter, file search, and the outline picker. Bare `j`/`k` have to reach the text in
  those modes (a path like `/home/jay` is untypeable otherwise), so the vim reflex gets the
  modifier rather than being sent to find the arrow keys.
- **Clicking a file no longer scrolls the list.** Scroll down, click something halfway up
  the screen, and the list used to yank itself so the clicked file sat on the very last
  row. The home screen has a real scroll offset now, and it moves only when the selection
  would leave the screen.
- **The home screen no longer changes what you have selected while it is scanning.** The
  cached list paints first and the live scan refines it; every batch that arrived re-sorted
  the list under a selection that was a bare row number, so a newly-found file appearing
  above your highlight quietly slid a *different* file under it — and pressing Enter at
  that moment opened something you never picked. The selection now holds onto its file and
  follows it wherever the sort puts it. Worst on a cold cache, a large tree, or a network
  mount, where the scan streams for longer.
- **Choosing a directory lands in the menu, not the filter.** It used to drop straight
  into filter mode, where the next keystroke silently hid files.

## 2026.8.20

- **Section folding.** `za` folds the section you're in behind its heading; `zM` folds
  everything — the document becomes its own table of contents — and `zR` opens it all
  back up. Clicking a heading toggles its fold too. A folded heading wears a dim `▸`
  and a trailing `…`; everything inside it, nested sections included, takes no rows.
  **A fold never hides anything from search**: matches inside folded sections still
  count, and jumping to one — or following a link, an outline entry, or a `#fragment` —
  unfolds its way there. Esc does not unfold; folds are deliberate.
- **A sticky heading breadcrumb.** Scroll deep into a long document and the enclosing
  sections stay pinned atop the page — `The Book ▸ Chapter One ▸ Detail` — with a rule
  under them, so "where am I" always has an answer. Too narrow a terminal drops the
  outermost sections first behind an ellipsis; the heading itself is never shown doubled
  when it is already the top visible line; documents with no headings never show a band
  at all. `B` toggles it live and the choice persists (`breadcrumb` in the config,
  default on).
- **Groundwork for the breadcrumb and section folding**: the core can now answer "which
  sections enclose this position" and "where does this section end" — derived from heading
  levels on demand, so both future features will agree on what a section is. No visible
  change yet.
- **Pipe into it.** `gh pr view | carrel`, `git show HEAD:README.md | carrel` — a piped
  document opens the reader, and it **streams**: a slow producer (an agent writing as it
  thinks) paints immediately, content appends as it arrives, and your reading position and
  search matches hold while it does, because positions never depend on the screen. The
  footer lamp says `streaming` until the pipe closes. `carrel -` forces stdin mode,
  `cmd | carrel - pattern` prints the match report, `carrel --plain - [W]` renders piped
  input as plain text, and piping *out* still yields plain text so pipelines pass through.
  Following a link out of a piped document and pressing `Ctrl-O` comes back to it, from
  memory — a pipe has no path to re-read.

- Packaging recipes stamped for 2026.8.17: the COPR spec, the AUR PKGBUILDs and
  `.SRCINFO`, with `carrel-bin` checksums filled from the real release artifacts —
  the first archives that ship the man page and completions.
- A picker test no longer fails when the build directory is deeper than the
  overlay is wide (found by the COPR builder's `/builddir/...` tree): the test
  asserted every painted path in full, where the painter's actual contract is
  to show as much of the path as fits. The test now pins that contract, long
  path included. The COPR spec carries the fix as a patch on the v2026.8.17
  tarball (release 2), to be dropped at the next release.
- **Install with Homebrew.** Every release now builds a Homebrew formula and ships
  it as `carrel.rb` alongside the binaries; it lands in
  [`VaHughes/homebrew-tap`](https://github.com/VaHughes/homebrew-tap):
  `brew install VaHughes/tap/carrel`, on macOS and Linux.
- **A nixpkgs package, prepared.** `contrib/packaging/carrel-package.nix` is the
  by-name package ready for a nixpkgs PR, minus the two hashes that need a machine
  with nix to compute.
- **README claims audited against the field.** Ten shipping terminal markdown
  readers were driven through the same resize-and-reflow search test carrel passes.
  One other reader keeps its matches intact, so "no shipping tool does this" was an
  overclaim and now reads "most readers lose or shift matches"; a stale star count
  became a hedge that won't age.

## 2026.8.17

- **A comfortable reading measure.** Prose now wraps at 90 columns and centres on the page
  instead of stretching across the whole terminal — past roughly ninety characters the eye
  loses its place on the return sweep. **Tables, code blocks, images and diagrams are
  unaffected** and still use the full width, so nothing that fits today starts wrapping or
  turning into cards. Terminals at or under 90 columns of text look exactly as they did.
  Configurable as `max_width` in the config file; `max_width = 0` turns it off.
- **The config file is documented** in the README for the first time, with all four keys.
- **Fedora packages.** `dnf copr enable vahughes/carrel && dnf install carrel` — Fedora 43 and
  44, x86_64 and aarch64, with the man page and shell completions installed.
- **Click a file on the home screen.** One click selects, a double click opens — the list
  looked clickable and silently wasn't. Clicking a search result works too, including its
  dimmed context line, and opens the file at the first match.
- **The directory picker is clickable too** — click to highlight, double click to choose.
  Clicking a file worked but the `d` overlay didn't, which was a gap the previous change
  created.
- **Time remaining.** The status bar now says how long is left to read alongside the
  percentage — an estimate at 200 words per minute, quiet under a minute and at the end.
- **Esc now clears search highlights.** After running a search and pressing Enter, Esc cleared
  the mouse selection and the selected link but left the match highlights on screen, with no
  way to get rid of them except running another search. It does not move you — accepting a
  search took you somewhere on purpose, so clearing the highlights is not an undo.

## 2026.8.16

- **Frontmatter renders as a card instead of a heading.** A file starting with
  `---` / `title: …` / `---` used to show an `<h2>` full of YAML — the first thing on
  screen for every Obsidian, Hugo, Jekyll, Zola and Quartz note. It now renders as a quiet
  metadata card with the keys aligned, and stays searchable. TOML (`+++`) too.
- **LaTeX math.** Display math (`$$…$$`) renders as box art — stacked fractions, roots with
  overbars, big operators with their limits, matrices inside stretched brackets. Inline math
  (`$…$`) renders in place, so `$E = mc^2$` reads as `E = mc²`. Anything too wide, or that
  will not parse, falls back to the LaTeX source rather than showing you a broken equation.
- **`m` now toggles math as well as diagrams** between rendered art and source.
- **Definition lists**, **superscript and subscript**, and **bare `www.` links** now render.
- **`carrel --version`** works. It used to exit 1 trying to open a file called `--version`.
- **A man page and shell completions** for bash, zsh and fish, in `contrib/`, installed by
  the AUR and RPM packaging.
- **A conformance suite** — `crates/carrel/tests/corpus/conformance.md` and its 15
  assertions — covering every construct carrel supports *and* every one it deliberately does
  not, so the README's claims are tests rather than promises.
- **A demo in the README** — the home screen, filtering to a file, and reading it. Recorded
  with [VHS](https://github.com/charmbracelet/vhs) from [`contrib/demo.tape`](contrib/demo.tape),
  so it can be re-rendered whenever the interface changes.
- **The README is explicit that the GUI does not exist yet.** "It ships in two forms" read as
  though both were available; carrel is a terminal application today, the GUI is planned and
  not written, and there is nothing GUI-shaped to download.
- **Corrected a stale status note** that called code blocks, tables and images "placeholders" —
  syntax highlighting, tables, images and mermaid diagrams have all shipped.

## 2026.8.12 — initial release

A terminal markdown reader, from scratch:

- **Search that survives reflow and terminal resize** — matches are byte offsets into the
  document, never screen positions, so nothing is lost when the window changes.
- **A home screen** that lists the markdown around you, newest first — type to filter, `/` to
  search inside every file, `d` to choose a directory.
- **Reading**: vim motions with counts, incremental search with smart case, an outline picker,
  links (OSC 8 hyperlinks, relative-link traversal with history, `[[wikilinks]]`), syntax
  highlighting, images (kitty protocol with half-block fallback), tables with an automatic
  card view when they're too wide, GFM alerts, footnotes, mermaid diagrams as Unicode box art.
- **17 themes**, cycled live with `T`, persisted. `NO_COLOR` honoured.
- **A contextual key-hint footer** that always answers "what can I press right now" — hide it
  with `H` or by clicking the lamp.
- **Mouse selection that copies clean text** — drag, double-click a word, triple-click a block;
  no quote bars or markers in what lands on your clipboard.
- **Live reload**, silent reading-position resume, a `--plain` mode for pipes and screen
  readers, and a scrollbar you can grab.
