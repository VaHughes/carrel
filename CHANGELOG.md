# Changelog

Versions are calendar dates, `YYYY.M.D` (Eastern time).

## 2026.8.21

- **The directory picker is an input now.** `d` opens a box you type a path into, and it
  completes against the filesystem as you go — `~` expands, a trailing `/` lists a
  directory whole, a bare word completes against the one you are standing in, and the
  matches redraw on every keystroke. The old fixed menu of `~/Documents` and
  `~/Documents/GitHub` is gone: those were a guess, and they found nothing at all on a
  machine that keeps its files anywhere else. With nothing typed the picker offers the
  current directory and the top level of your home. `$HOME` itself is still never offered
  — scanning all of it descends into every cache and container on the machine — but you
  can type it. Arrows and `Ctrl-N`/`Ctrl-P` move; `Esc` clears the path, then closes.
- **Clicking a file no longer scrolls the list.** Scroll down, click something halfway up
  the screen, and the list used to yank itself so the clicked file sat on the very last
  row. The home screen has a real scroll offset now, and it moves only when the selection
  would leave the screen.
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
