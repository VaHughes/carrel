# Changelog

Versions are calendar dates, `YYYY.M.D` (Eastern time).

## Unreleased

- Packaging recipes stamped for 2026.8.17: the COPR spec, the AUR PKGBUILDs and
  `.SRCINFO`, with `carrel-bin` checksums filled from the real release artifacts —
  the first archives that ship the man page and completions.

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
