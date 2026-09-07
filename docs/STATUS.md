# Status

Last updated: 2026-09-07

## Current state

- **Latest release: v2026.9.3** (the click-first release: link clicks, fold-marker buttons,
  right-click and `≡` menus, footer chips, hover, the clickable path row, `⌂`, `--no-mouse`).
  Published to crates.io (both crates), GitHub releases (six unix targets + shell installer),
  Homebrew tap, Fedora COPR (F43/F44, x86_64 + aarch64).
- **Unreleased on `main`:** `merman-core` and `merman-ascii` are now pinned exactly beside
  `merman` (commit `95a3a0d`) so `cargo install carrel` builds again without `--locked`
  after upstream published `merman-ascii 0.8.0-alpha.6` on 2026-09-02. Prebuilt channels were
  never affected. This needs a release to reach crates.io; until then the README says
  `cargo install carrel --locked`.
- Working tree is clean. `cargo test --workspace`: **761 tests, all green** (2026-09-07,
  rustc 1.98.1 locally; CI pins 1.97, MSRV 1.95).
- Feature-complete for the terminal reader as planned; the roadmap in `README.md` is the
  authoritative done/open list.

## Recently completed

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

Features still open on the README roadmap: marginalia, search results as a document, image
lightbox, word-level diff color, tags browser (deliberately not built — `/rust` on the home
screen already retrieves; a browser adds discovery, wanted only if the vault persona is
confirmed), wide-table horizontal scrolling, hyphenation at narrow measures.

Upstream: delete `render::declare_wide_cells` when ratatui#2721 (fix for #2651) lands.

The GTK4 GUI phase is not started and has no date. Its remaining homework (in the private
notes' `research.md`): Q29 (dual-frontend prior-art study, P0) and Q33 (P1), to be answered
before that phase rather than during it.

## Recent decisions

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
