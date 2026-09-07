# Architecture

Carrel is a markdown **reader** (not an editor) built as one shared core crate and one
terminal frontend, with a GTK4 + WebKitGTK frontend designed for but not written.

```
crates/carrel-core/   document model · search · layout primitives.  NO UI DEPENDENCIES, EVER.
crates/carrel/        the terminal frontend (ratatui).  The only frontend that exists today.
                      A GTK4 + WebKitGTK frontend will sit ALONGSIDE it, never inside it.
```

The full spec (position types, data structures, algorithms, the resize path, a 20-item
"do not do this" list keyed to failures in real editors) is `architecture.md` in the private
notes repo, checked out at `../carrel-notes` on the maintainer's machine. Read it before
touching `carrel-core`. Design specs and plans for each feature slate live there too, under
`docs/superpowers/{specs,plans}/`.

## The rule that outranks everything

> There is exactly one authoritative coordinate space: a byte offset into a flattened,
> unwrapped display text (`Document::text`). Screen row, wrap column, and highlight rectangle
> are *derived* functions of `(document, width)` — recomputed on resize, never stored. A
> search hit recorded at width 80 is bit-for-bit the same value at width 40.

Everything follows from that sentence. Search state cannot be invalidated by reflow because no
search state is ever expressed in display coordinates. That is what makes the headline feature
(search that survives reflow and resize) possible, and why the two mdfried bugs this project
exists to not have (#52, #53) cannot occur here.

## The non-negotiable disciplines

Every project surveyed during research that planned a second frontend for "later" never got
one — the first frontend's assumptions calcified into the core (Helix's own architecture doc
records exactly this). Hence:

1. **No sized layout type in `carrel-core`'s public API.** `fn height() -> u16` is the shape
   that killed Helix's view layer. `Node::indent: u16` is fine — it is width-*independent*.
2. **Positions are byte offsets.** Never line numbers, never pixels. Scroll position is a
   `DocByte` anchor; the row is a derived cache.
3. **The core emits semantic scopes and tokens, never ANSI and never RGBA.** Each frontend maps.
4. **Share an `Action` intent enum**, bind keys and mouse per frontend. Keys and pointer
   events both become `Action` values before the state machine sees them.
5. **Never let the TUI dictate a core type.**
6. **The TUI's state layer is ratatui-free.** `action`, `app`, `layout`, `view`, `plain`,
   `config`, `scan`, `home`, `images`, `state`, `wiki`, `grep`, `diagrams`, `footer`,
   `breadcrumb`, `menu` never import ratatui, so behavior is tested with no terminal and a
   GTK frontend can reuse them verbatim.

`./scripts/check-discipline.sh` enforces 1–4 and 6 mechanically (UI crates, ANSI escapes,
width-dependent public API, char/byte conversion, ratatui in the state layer). It has been
verified to fail on injected violations. It skips comment lines, because the docs necessarily
name what the rules forbid. Color (rule 3's RGBA half) is a convention, not a check.

## Decisions already made — do not relitigate

Each was researched at length; the reasoning and runner-up are recorded in the private notes
(`idea.md`, `architecture.md`, `NAMING.md`). Open an issue with new evidence before a PR that
reopens one.

| Decision | Outcome |
|---|---|
| Language | **Rust.** Runner-up Zig (no byte-offset parser, pre-1.0 churn) |
| Parser | **pulldown-cmark**, for per-event byte offsets *including inlines*. comrak gives line/column only and was reversed out |
| Architecture | **One core crate + independent frontends**, seam **below** layout |
| GUI toolkit | **GTK4 shell + WebKitGTK 6 content view** (`gtk4-rs` + `webkit6`), floor WebKitGTK 2.44 |
| Sequencing | **Terminal first**, GUI designed for from day one; not started, no date |
| Scrollbar vs O(1) resize | **Keep the scrollbar.** Eager O(N) height pass behind a 40 ms debounce — confirmed by measurement (11 ms/MB ASCII, 31 ms/MB CJK; resize 15.4 ms per `App::on_resize` on a 1 MB corpus). The estimate-then-refine fallback stays unwritten |
| Highlighting | **syntect**, not tree-sitter (0.8 ms load vs 37.9 ms per language). `regex-fancy` engine, because `regex` is already linked and onig's C toolchain breaks on GCC 15 / Fedora 42 / musl / wasm |
| Text storage | **`String` + line-start index. No rope** — the document is read-only |
| Windows | **Declined** (nobody here can field-test a Windows terminal). Six unix targets are the full set |
| Remote URLs / fetching | **Never.** No HTTP client, no TLS library in the tree; `cargo tree` is the audit. Clicking a URL copies it; carrel never spawns a browser |
| GUI search mapping | Settled: the core search API is frozen as byte ranges, which is exactly what a DOM-range mapping needs (segment table from a core HTML emitter, CSS Custom Highlight API) |
| Row cache | **None, on purpose.** Re-wrapping visible blocks costs ~40 µs a frame. Do not add one without a measurement that demands it |

## carrel-core

| Module | What it is |
|---|---|
| `position.rs` | `SrcByte` / `DocByte` / `NodeId` / `BlockIdx` / `Affinity`. Every position is a u32 byte offset by design (`cast_possible_truncation` is allowed for this reason; non-position narrowing casts use `u16::try_from`). |
| `document.rs` | The display-text model with the **doc-to-source provenance table** (`Prov`) — the part no editor has, because editors have source space == doc space and a markdown reader does not. Paragraphs, headings, code, lists, quotes, tables, rules, inline style, frontmatter metadata cards, definition lists, footnotes, GFM alerts, `<details>`, wikilinks, `www.` autolinks, attached `^sup^`/`~sub~`. The pulldown-cmark event match has **no catch-all**: a parser bump that adds a variant fails to compile rather than silently dropping a construct. |
| `search.rs` | Complete per spec. `Matches` holds no row/column/width/block. `intersecting()` implements the wrap-affinity rule as two half-open comparisons. `content_pattern` keeps grep (home-screen `/`) and reader matching identical. Measured: 648 µs for a 3-char literal over 1 MB; 0.74 ms per keystroke. |
| `layout/` | The reflow layer. `units.rs` = all Unicode (UAX #14 break units, measured, width-independent); `pack.rs` = all fitting (never sees a string, so its invariants are testable against hand-built units); `mod.rs` = public API and the 64 KiB chunker. **Every block wraps through the same `wrap`** — what differs is the fit: code carries a continuation marker + hanging indent, tables arrive pre-aligned from parse, images/mermaid/math wrap their alt text while their real heights are a frontend override (`block_rows`). `proptests.rs` holds the property tests. |
| `highlight.rs` | syntect scopes classified into semantic `TokenKind`s (including `Inserted`/`Deleted`/`Meta` for diffs). **Lazy** — `Document::tokens(b)` on first paint; parse-time highlighting would cost a code-heavy document ~100 ms. Two-pass classification (containers before innermost specificity); use `Scope::is_prefix_of`, never `build_string`. |
| `math.rs` | LaTeX via `pulldown-latex` (pinned `=0.8.0`) to a cell-free `MathExpr`. Inline math enters `Document::text` already rendered — the display text is authoritative. |
| `diff.rs` | Turns a unified diff or `git log -p` into markdown — a heading per commit and per file, hunks as `diff` fences — so folding, breadcrumb, outline and search work on diffs with no new code. Detection never touches a `.md` file. |

Key facts that are easy to get wrong:

- **Table alignment happens at parse, not at paint.** Column widths are max-content display
  widths (width-independent), cells pad with synthetic spaces in the display text, every visual
  row is one contiguous doc range. The `│` separators are paint-time decoration. The synthetic
  `\t` between cells marks a cell boundary; the card view finds cells through `Table::cell_starts`.
- **Tabs are expanded at parse, not at layout** — a tab's width depends on its column, which the
  per-cluster `WidthFn` cannot express.
- **`Event::Text` is not always a source substring** (entity decoding, smart punctuation change
  byte lengths). This is why `Prov` exists; `doc = src - block_start` is false.
- **pulldown-cmark emits no `Paragraph` inside a tight list item**, so `NodeKind::Item` is a
  leaf. The `every_byte_of_display_text_is_covered_by_a_block_or_a_separator` test catches
  mistakes here.
- **`Node::parent` is `None` for every node, deliberately.** The container pass waits for its
  real consumer (the GUI HTML emitter). Section ancestry never needed it: breadcrumb and folding
  use `Document::section_path` / `section_end`, derived per call from heading levels.
  `quote_depth` is a field for the same reason.
- **Quote bars are not a prefix.** A prefix is first-row-only (list markers); a quote bar
  repeats on every row.
- **`avail` is per logical line, not per block.** `pack` takes a `LineFit` with two budgets.
  Do not hand `pack` the text or the width function — that seam keeps the packer testable.
- **`ENABLE_DEFINITION_LIST` eats `:::` directives** — a deliberate trade, recorded by a
  conformance test.
- **Every hand-rolled inline pass must check `in_code_block()`** and must assume the text run
  it wants may arrive split across two `Text` events (upstream splits at a delimiter run it then
  declines).
- **After reflow the anchor is usually mid-row.** The row containing it begins earlier at the
  new width. That is correct, not drift.

## carrel (the TUI)

| Module | What it is |
|---|---|
| `main.rs` | Entry, `USAGE`, the event loop (`run_loop`, shared by file and piped entries), `paint` (every frame inside a DEC 2026 synchronized update plus the OSC 8 post-draw pass), mtime `Reloader`, rescan timer, signal handling. |
| `app.rs` | The state machine — `App`, `update()`, folding, bookmarks, links, `reveal_byte`, `adapt` (the single funnel every parse goes through for diff detection), `text_size`/`text_x`/`text_y`. |
| `action.rs` | The shared `Action` intent enum and the per-frame click-target registry (`Targets`). |
| `keys.rs` | Vim motion set with a count register; help tables and footer hint tables with drift guards. |
| `view.rs`, `layout.rs` | Viewport (a `DocByte` anchor) and the frontend-side layout (`Layout::with_hidden`, `block_width(kind)` — exhaustive match over `NodeKind`). |
| `render.rs` | Paints rows into the `Buffer`; never uses `Paragraph`/`Wrap`. Highlights by `Buffer::set_style` over a rect, never by splitting spans. `declare_wide_cells` works around ratatui#2651. |
| `theme.rs` | The only file with a color. 17 palettes plus `omarchy` (derived from the desktop's `colors.toml`, `omarchy.rs`). |
| `home.rs`, `scan.rs`, `grep.rs`, `fuzzy.rs` | The home screen: streamed `.gitignore`-aware scan (`ignore` crate, `require_git(false)`), cached index, 2 s rescan while listed, directory picker with path completion and remembered places, fuzzy filter, multi-file content search, frontmatter titles. |
| `menu.rs`, `footer.rs`, `breadcrumb.rs` | Pure selectors for the right-click/`≡` menus, the lamplight hint row, and the sticky heading band. |
| `state.rs`, `config.rs` | XDG state (reading positions, bookmarks) and XDG config. Both are injected as `Option` dirs (`None` in constructors) so tests can never reach the real files. |
| `stream.rs` | stdin on a thread with UTF-8 carry across chunks; keys arrive via `/dev/tty` (crossterm's native fallback). |
| `images.rs`, `diagrams.rs`, `math_art.rs`, `wiki.rs`, `links.rs`, `plain.rs`, `ansi.rs` | Image sizing (kitty/halfblock via `ratatui-image`), mermaid box art (`merman`), TeX-lite math boxes, `[[wikilink]]` resolution, forward/backlinks with no index, `--plain` and `--render` output. |

Key facts:

- **Geometry is either inverted or recorded, never re-derived.** If a thing's position depends
  only on `(cols, rows, flags)`, invert the geometry function that placed it (text body,
  scrollbar thumb, home list rows). If it depends on the data drawn (footer chips, path-row
  segments, links, fold markers, pane rows), the paint pass pushes its rectangle into `Targets`
  and the event loop reads it back. `every_registered_target_covers_the_thing_it_acts_on`
  guards the registry.
- **`PAD_LEFT` is the minimum margin, not the left edge.** Prose is centered at `max_width`;
  `App::text_x(cols, max_width)` is the real left edge and `App::text_y()` the top edge. Paint
  and hit-testing both go through them.
- **Two width budgets.** `text_size` returns `(prose, bleed, height)`; `text_w()` is the prose
  measure, `bleed_w()` the full area. `table_overflows` must get the bleed width.
  `paint_rows` takes the full area and shadows a per-block rect inside the loop.
- **`App::reveal_byte` is the one gate** for every byte-targeted jump — a fold must never make
  a destination unreachable. Hidden blocks are zero rows and zero gap; `paint_rows` must skip
  them explicitly.
- **Notes go to the screen that paints them** — `App::set_note` routes between `App::note`
  and `Home::note`.
- **OSC 8 is a post-draw pass** (ratatui/crossterm have no hyperlink support); it reads the
  finished frame so overlays and theme colors are respected. URLs are stripped of control
  characters at collection.
- **Never call `ratatui_image::Picker::from_query_stdio`** — its query thread steals stdin.
  Font size comes from `TIOCGWINSZ`, protocol from the environment. Pixels never enter the core.
- **Sub-image scroll clipping is deferred** (top-anchored crop) until the GUI's image work.
- **Selection is a doc-byte range**; `cluster_at_col` in core is the pointer-hit inverse of
  `cols_for_doc_range`. Copy goes out via OSC 52.

## Pinned dependencies — bump with care

| Crate | Pin | Why |
|---|---|---|
| `unicode-width` | `=0.2.2` | A bump silently changes reflow. Measure the **cluster string**, never sum per-char widths. Helix pins `=0.1.12` for the opposite reason. |
| `merman`, `merman-core`, `merman-ascii` | `=0.8.0-alpha.5`, all three | Alpha line breaks API between alphas; merman's own sibling deps are caret ranges, so an unlocked `cargo install` drifted to alpha.6 and failed (2026-09-02). `ascii` feature only — `png` costs +12 MiB. Bump all three together after a scratch-crate fidelity check. |
| `pulldown-latex` | `=0.8.0` | Same rule as `unicode-width`. |
| `ratatui-image` | default features **off** | Defaults drag in libchafa + pkg-config. |
| `image` | codecs listed explicitly | The default set enables every format; this project counts binary bytes. |
| `regex` | — | The biggest binary cost (+1.59 MiB), which is what makes syntect's `regex-fancy` free. |

Binary is ~10.5 MiB release; merman's ascii tier is +6.8 MiB of that (an accepted cost).

## Security posture

A markdown file is untrusted input. Carrel fetches nothing, spawns nothing, follows no
symlinks during the walk, reads no ignore file above the root, canonicalizes link targets and
asks for a second Enter before opening one outside the library, strips control characters
from URLs before OSC 8, and honors `NO_COLOR`. The index cache holds paths and mtimes only.
