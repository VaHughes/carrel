# Carrel — agent entry point

Carrel is a free and open-source markdown **reader** for the terminal (not an editor): a
Rust workspace with `crates/carrel-core` (document model, search, reflow layout — no UI
dependencies, ever) and `crates/carrel` (the ratatui frontend). A GTK4 + WebKitGTK frontend
is designed for but not written. Search survives reflow by construction because every
position is a byte offset into one flattened display text. Published to crates.io, GitHub
releases, Homebrew and Fedora COPR under CalVer.

**Read `docs/STATUS.md` first.** Docs are updated via `/wrap-up` at the end of each session.

- `docs/STATUS.md` — current state, what shipped recently, what is open, recent decisions (volatile).
- `docs/ARCHITECTURE.md` — the one-coordinate-space invariant, the discipline rules, decided
  questions, module map, pinned dependencies, hard-won facts about the code.
- `docs/DEVELOPMENT.md` — toolchain, the five gates, testing tiers and guards, conventions,
  release gotchas, config/state locations.
- `README.md` — for humans: what it is, install, usage, configuration, roadmap.
- `CONTRIBUTING.md` — the invariant, the gates, how changes happen here.
- `CHANGELOG.md` — per-release notes; the top heading is per release, new work goes under `## Unreleased`.
- `contrib/packaging/README.md` — packaging recipes, COPR, `.SRCINFO`, `release.yml` rules.
- `CLAUDE.local.md` (gitignored) — machine-local notes: tool inventory, credential locations,
  the private notes checkout (`../carrel-notes`: specs, plans, research, `RELEASING.md`).
