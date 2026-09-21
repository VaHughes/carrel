# Development

## Toolchain

Rust via rustup, edition 2024, workspace resolver 3. CI pins **1.97** for the gates and checks
the declared MSRV **1.95** (`rust-version` in `Cargo.toml`, set by `merman`'s floor) with
`cargo check --workspace --all-targets --locked`. Keep the two toolchain pins in `ci.yml` and
`release.yml` equal; bump them deliberately with the local toolchain (clippy runs with
`-D warnings`, so a moving `stable` would break CI on days nobody changed anything).

rustup was installed with `--no-modify-path` on the maintainer's machine, so `cargo` may not be
on `PATH` in a fresh shell: `. "$HOME/.cargo/env"`.

## Commands

```bash
cargo test --workspace                                  # every test; keep green
cargo clippy --workspace --all-targets -- -D warnings   # zero warnings is the bar
cargo fmt --all --check
cargo check --workspace --all-targets                   # benches still compile
./scripts/check-discipline.sh                           # the architectural guard
./scripts/check-packaging.sh                            # recipes vs what ships, .SRCINFO vs
                                                        # PKGBUILD, README links as crates.io
                                                        # rewrites them, every recipe's version
                                                        # vs Cargo.toml; pass an archive path to
                                                        # check a REAL release artifact
cargo bench -p carrel-core --bench wrap                  # reflow throughput
cargo bench -p carrel --bench resize                     # end-to-end resize latency
cargo run -p carrel -- <FILE> [PATTERN]
cargo run -p carrel                                     # the home screen
vhs contrib/demo.tape                                   # re-render assets/demo.gif (needs vhs + ttyd)
```

The first five (test, clippy, fmt, discipline, packaging) plus `cargo check` are exactly what
`.github/workflows/ci.yml` runs on `main` and on pull requests; `release.yml` runs the same
gate job against the tagged commit before building anything.

When scripting the gates, run each as its own `;`-terminated command with an echoed marker and
gate on exit codes: `set -e` does not abort on the left side of `&&`, and `grep -c` exits 1 on a
zero count, which is the *clean* clippy case. After any multi-part scripted edit, grep the
written file for each piece — a mid-script abort has silently dropped tests and a paint arm
before.

## Running your own build

Symlink, never copy — a copied binary goes stale on the next rebuild while still answering to
`carrel`:

```bash
cargo build
ln -sfn "$PWD/target/debug/carrel" ~/.local/bin/carrel
```

`cargo clean` or a failed build breaks the link until the next successful build. The debug
build is ~20x the size and noticeably slower on a large library; point the link at
`target/release/carrel` when judging performance. The maintainer field-tests continuously, so
rebuild after every user-facing change.

## Testing

Four tiers: pure state (no terminal), `TestBackend` frames, property tests (proptest, for the
reflow layer and resize), and the automated pty smoke (`crates/carrel/tests/pty.rs`).

- **Tests first.** Every bug fix starts with a test watched failing against the current code.
  A property test that passes on its first run may be asserting nothing — break the
  implementation on purpose and watch it fail. Break-check every guard in both directions; a
  guard that errors out is a guard that passes (a `grep -E … -P` once reported ✓ while
  testing nothing).
- **Tests must never reach the real config or state.** `App::config_dir` and `App::state_dir`
  are `None` in constructors; `main.rs` sets them. A test once drove the real
  `config::save_root()` and every `cargo test` wrote `root = /tmp/...` into the developer's
  config.
- **pty smokes** set `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` to a scratch
  dir. To exercise the real binary without a TTY:
  `( sleep 1; printf 'q' ) | script -qec "stty rows 20 cols 76; ./target/debug/carrel README.md" /dev/null`
  and check the output both enters (`ESC[?1049h`) and leaves (`ESC[?1049l`) the alternate
  screen. A detached pty reports 0x0 unless you set `stty` size. `TestBackend` will never
  tell you the loop spins at EOF; `carrel` refuses the TUI unless stdin and stdout are terminals.
- **Installer smokes** additionally override `HOME`. The dist shell installer writes four
  dotfiles (`~/.bashrc`, `~/.profile`, `~/.zshrc`, `~/.config/fish/conf.d/carrel.env.fish`);
  grep all four for the scratch path before and after.
- **Drift guards that fail the build:** help overlay vs keymap (exhaustive match in
  `keys.rs`), footer hints (an honesty test presses every hinted key through the real
  dispatcher — extend `probe()` when adding a hint), menus (shortcuts come from the keymap),
  man page vs help overlay and vs `.SS Mouse` (`tests/pty.rs`, `NOT_IN_MAN` is the
  deliberate-exemption list), completions vs `USAGE`, click targets
  (`every_registered_target_covers_the_thing_it_acts_on`), emoji cell widths
  (`tests/wide_cells.rs`), click geometry
  (`tests/measure.rs::clicking_a_column_lands_on_the_character_painted_there`).
- The conformance corpus is `crates/carrel/tests/corpus/*.md` with `conformance.rs`.
- **Look at the thing.** A wide table once lost its last two columns with every frame test
  green; it was found by reconstructing a real pty screen.

## Conventions

- **Measure before optimizing.** Profile before rewriting an algorithm; the first reflow cut
  was 50 ms/MB and the cost was allocation, not line breaking.
- **Decided questions stay decided** unless new evidence arrives (`docs/ARCHITECTURE.md`).
- **A new clickable thing registers itself; it is never re-derived** (`Targets` in `action.rs`).
- **A new key goes in the help tables, the man page (`contrib/carrel.1`), the completions,
  and the menus** — the guards above will tell you which one you forgot.
- **One word per idea in anything the reader sees.** The 2026-09-21 pass settled the
  vocabulary: *collapse*/*expand* (never fold), *folder* (never directory, library, root,
  place or level), *heading bar* (never breadcrumb), *hint row* (never key hints or hint
  footer), *bookmarks* (never marks), *focus* (never spotlight), *text width* (never
  measure), *drawn ↔ text* for diagrams and math, *cards ↔ columns* for wide tables, *keep
  up with the end* (never follow — which means following a *link*). Internal identifiers
  keep their own names (`FoldToggle`, `library_root`, `breadcrumb.rs`); this rule is about
  strings, help rows, menu labels, notes, the man page and the README. **Nothing enforces
  it** — there is no compiler for prose — so check new strings against this list by hand.
- **`theme.rs` is the only file with a color.** Diff scopes go in the *container*
  highlighting pass, not the ordered one.
- **`block_rows` carries images, mermaid and math**; extend its `debug_assert`, never delete it.
- **Update `CHANGELOG.md` in the same change.** The top heading is per *release*: after a tag,
  new work goes under `## Unreleased`. Versions are CalVer `YYYY.M.D` (Eastern time,
  `TZ=America/New_York date +"%Y.%-m.%-d"`), which allows one release per day.
- **Update the markdown in the same commit as the change that dated it.** Stale docs are
  worse than missing ones. Dated records (`CHANGELOG.md`, specs, plans) are history and are
  not rewritten.
- US English in prose and code; existing identifiers keep their spelling.
- Committing and pushing finished work on `main` is standing permission; ask before anything
  that rewrites history or changes repo visibility. **Watch CI by SHA**, never
  `gh run list -L1` (it races and returns the previous commit's green run): resolve the run id
  from `headSha == $(git rev-parse HEAD)` first.

## Configuration and state (for reference)

- Config: `$XDG_CONFIG_HOME/carrel/config`, `key = value` lines. Keys: `max_width` (90),
  `theme`, `hints` (true), `titles` (false), `outline_margin` (false), `breadcrumb` (true),
  `mouse` (true), `root`, `place` (repeats, newest first, capped at eight). Unknown keys ignored.
  The settings pane (`,`) shows all but `mouse`, `root` and `place` with their live values and
  prints the file's path; each row writes through the one existing writer for that setting, so
  the pane never becomes a second place persistence happens.
- State: `$XDG_STATE_HOME/carrel` — reading positions (`permille`, `words`; the old 3-field
  form still parses) and bookmarks, per document.
- Cache: `$XDG_CACHE_HOME/carrel` — the home-screen index (paths and mtimes only).
- `READING_WPM` (200) is deliberately not configurable; `RESCAN_EVERY` is 2 s.

## Releasing

The ordered runbook is `RELEASING.md` in the private notes repo. Its shape: documentation
sweep (step 0 — grep for version strings, moved keys, counts, roadmap entries) → local gate →
version bump **and recipe version stamps in the release commit, before the tag** →
`cargo publish` core first → tag `v{version}` → watch CI by SHA → verify under a scratch
`HOME` → checksum stamps → Homebrew formula hand-pushed to `VaHughes/homebrew-tap` →
COPR `buildscm`. Packaging details, the COPR command that actually works, `.SRCINFO`
regeneration and the `release.yml` hand-edit rules are in `contrib/packaging/README.md`.

Gotchas:

- `dist` **flattens every `include` entry** to the archive root (`contrib/carrel.1` → `carrel.1`).
  Nothing in CI builds the packaging recipes against a real archive; run
  `./scripts/check-packaging.sh <archive>` on the downloaded artifact before publishing a
  binary package.
- `release.yml` is hand-edited (SHA-pinned actions, a `gate` job). `dist-workspace.toml`
  has `allow-dirty = ["ci"]`; **never run `dist init`**, which unpins and deletes the gate.
  `dist init` also deletes every comment in `dist-workspace.toml`.
- **Relative links in the README break on crates.io** (resolved against `repository` +
  `path_in_vcs`). Use absolute URLs; the rendered readme is baked at publish time.
- `copr-cli status` on a running build blocks past a ten-minute tool timeout; poll in bounded
  batches. `copr-cli` may need a venv (`python3 -m venv … && pip install copr-cli`).
- The x86_64-apple-darwin release job once failed with `dist: command not found` and passed
  on rerun; if it recurs, pin the installer step.
- The README demo is a committed VHS script (`contrib/demo.tape`), not a recording. VHS
  `Type` strings cannot contain `\"`; the staging block must stay under `Hide` and export all
  three XDG dirs into a scratch dir before `carrel` starts.

Machine-local notes (tool inventory, credential locations, the private notes checkout) live
in the gitignored `CLAUDE.local.md`.


## Ghostmachine CI runners

Owner pushes, same-repository PR checks and x64 Linux release jobs use the
separate `ghost-ci-public` Ubuntu 24.04 VM on ghostmachine. Labels are
`[self-hosted, linux, x64, ghostmachine, build]`. Build jobs serialize inside
this VM; its filesystem, credentials and network are separate from private CI
and production. The runner has no host filesystem mounts or LAN access.

Native macOS and ARM release builds remain on free GitHub-hosted runners.
External-fork PR jobs also use GitHub-hosted runners. Repository settings require
approval for **all external contributors**, including returning contributors;
this setting is essential because a fork can edit its own workflow routing.
Before approving a fork workflow, inspect its full workflow changes and ensure
no job targets `self-hosted`/`ghostmachine`. Do not approve a fork run that routes
code into the persistent VM. Workflow `if`/`runs-on` expressions alone are not a
security boundary against a modified fork workflow.

x64 release builds use a digest-pinned Ubuntu 22.04 container to preserve the
previous GNU libc baseline. Do not remove it when changing runner labels.
`release.yml` remains hand-edited: preserve runner routing, the container and
its dependency setup alongside the release gates and action SHA pins when
upgrading cargo-dist. `dist plan` retains all six release targets. Operations:
`work-bench/ghostmachine/ci-runners.md`.
