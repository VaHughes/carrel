# Status

Last updated: 2026-09-21

## Current state

- **Latest release: v2026.9.3** (2026-09-03 — the click-first release: link clicks,
  fold-marker buttons, right-click and `≡` menus, footer chips, hover, the clickable path
  row, `⌂`, `--no-mouse`). Published to crates.io (both crates), GitHub releases (six unix
  targets + shell installer), Homebrew tap, Fedora COPR (F43/F44, x86_64 + aarch64).
- **A release is owed.** Twelve commits sit unreleased on `main`, and the oldest that
  matters is a fix for a broken install: `merman-core` and `merman-ascii` are pinned exactly beside
  `merman` (`95a3a0d`) because upstream published `merman-ascii 0.8.0-alpha.6` on
  2026-09-02 and the caret ranges let it resolve in, so **`cargo install carrel` fails to
  compile for every version on crates.io** until this ships. Prebuilt channels — the
  installer, Homebrew, COPR, the AUR recipes — build from the lock file and were never
  affected. The README says `cargo install carrel --locked` in the meantime.
- Unreleased, newest first: **the beginner slate** (`efa4d81`, `c53c246`, `aea46b3`,
  `3e5a4dc`) — plain words throughout, the keys a beginner presses, honesty about
  what carrel cannot verify, a settings pane on `,` and a built-in first document on
  `carrel --tutorial`; dead-end buttons and the code-block `[copy]` chip (`7ce0b53`);
  the filterable help sheet (`c8ee8c2`); clickable breadcrumb segments (`721b117`); the
  library-browser picker and search results as a document (`0d11ad2`); the documentation
  restructure (`91f1816`); the merman pin (`95a3a0d`).
- Working tree is clean. All five gates green as of 2026-09-21: **798 tests**, clippy
  zero-warning, `cargo fmt --check`, `check-discipline.sh`, `check-packaging.sh`.
- Feature-complete for the terminal reader as planned; the roadmap in `README.md` is the
  authoritative done/open list.

## Recently completed

- Unreleased (2026-09-21) — **the beginner slate**, from an audit of what a
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
- Unreleased (2026-09-16 / 09-21) — the library browser replacing the path-prompt picker,
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

1. **nixpkgs by-name PR** — `contrib/packaging/carrel-package.nix` is version-stamped but
   still needs its two hashes (`docker run --rm nixos/nix`; steps in the file header).
2. **AUR** — recipes stamped and build-tested (`makepkg -f`), blocked on Arch reopening
   account registration (closed since aurweb v6.5.0 during a supply-chain campaign).
3. **Catalog listings** — awesome-ratatui, Terminal Trove, awesome-cli-apps / awesome-tuis.
4. `.deb` packaging (README roadmap).
5. Optional: set `HOMEBREW_TAP_TOKEN` and restore `publish-jobs = ["homebrew"]` to
   re-automate the formula push (currently hand-pushed each release).

Features still open on the README roadmap: marginalia, image lightbox, word-level diff
color, tags (a browser is deliberately not built — `/rust` on the home screen already
retrieves; a browser adds discovery, wanted only if the vault persona is confirmed),
wide-table horizontal scrolling, hyphenation at narrow measures.

Upstream: delete `render::declare_wide_cells` when ratatui#2721 (fix for #2651) lands.

The GTK4 GUI phase is not started and has no date. Its remaining homework (in the private
notes' `research.md`): Q29 (dual-frontend prior-art study, P0) and Q33 (P1), to be answered
before that phase rather than during it.

## Recent decisions

- 2026-09-21: **clicks over keybindings where a choice exists.** The beginner
  slate adds bindings only for keys that were already unbound, and reaches for
  a painted, clickable affordance everywhere else. `?` and `Ctrl-F` keep their
  vim/less meanings for this reason. This refines, and does not overturn, the
  2026-09-02 call that keybindings stay as they are: that one was about
  relocating existing commands behind `Alt+`, which is still not happening.
- 2026-09-07: documentation restructured into `AGENTS.md` + `docs/`; `CLAUDE.md` is now just
  `@AGENTS.md`. Note the public/private split: the old `CLAUDE.md` was gitignored because the
  maintainer keeps narrative/strategy material in the private notes repo. These new docs were
  written to be publishable (how-to and code facts), but whether to commit `AGENTS.md` and
  `docs/` or gitignore them is the maintainer's call.
- 2026-09-03: `merman` sibling crates pinned exactly (see above).
- 2026-09-01: `release.yml` hand-edited (gate job, SHA-pinned actions); `dist init` is
  forbidden; recipe version stamps move into the release commit.
- 2026-08-22: clicking a URL copies it, never opens a browser; Windows declined (Q19);
  remote URLs declined (written into the README as a position).
