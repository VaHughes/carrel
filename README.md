<div align="center">

<img src="https://raw.githubusercontent.com/VaHughes/carrel/main/assets/logo-pixel.png" alt="Carrel" width="220">

# carrel

**A quiet place to read your markdown.**

</div>

> **carrel** *(n.)* — a small enclosure with a desk, built for one person to sit and read.
> First recorded in the 13th century in the cloister of Westminster Abbey; found in libraries ever since.

Carrel is a free and open-source markdown **reader**. Not an editor, not a workbench — a reading desk.
It opens showing you the documents around you, renders them properly, and has a search that actually works.

**Carrel is a terminal application today, and that is the only version that exists.** A native GTK4
GUI is planned, and the codebase is deliberately built so one can be added without rewriting the
core — but it is **not written yet**. There is no GUI build to download, no preview, and no date.
Everything below installs the terminal reader.

<div align="center">

<img src="https://raw.githubusercontent.com/VaHughes/carrel/main/assets/demo.gif" alt="Carrel listing the markdown files in a directory, filtering to README.md, and scrolling through it" width="800">

</div>

> **Status: early, but it runs.** `carrel` shows you what is around you to read; `carrel FILE` opens
> a reader with vim motions, incremental search, and a resize that keeps your place. Syntax
> highlighting, tables, images and mermaid diagrams all render. See [Roadmap](#roadmap).

### It never fetches anything

Carrel reads only the directory you point it at. The default is the directory you are standing in;
anything wider is a root you choose yourself — from the list `d` offers, or by typing a path. It
sends nothing anywhere — remote images in documents are never fetched; they render as their alt
text. The index it caches lives under `$XDG_CACHE_HOME/carrel` and holds file paths and
modification times, nothing else.

That first sentence is **enforced, not merely intended**. A markdown file is untrusted input — a
shared vault, a downloaded README, anything you did not write — and a link in one can name any
path on the machine. Links resolving inside your library follow as they always have; one
resolving outside names the path and waits for a second Enter, so leaving is something you do
rather than something a document does to you. Both paths are canonicalised, so a symlink out of
the tree is caught too. The walk itself never follows symlinks and never reads an ignore file
above your root.

**This is a decision, not an omission.** Other readers will fetch a URL you hand them, or
pull a GitHub README, and that is genuinely convenient. Carrel will not, and is not going
to: a document is a thing you already have, and a reader that opens network connections on
your behalf is a reader you have to think about before you point it at something. Carrel
depends on no HTTP client and no TLS library — `cargo tree` is the whole audit — so this is
closer to a property you can check than a promise you have to trust. If you want a remote document, fetch it with a tool whose
job that is and pipe it in — `gh pr view 128 | carrel` — which keeps the fetching, and the
credentials it uses, somewhere you can see them.

---

## Why

Carrel targets what terminal markdown readers mostly haven't shipped:

| | |
|---|---|
| **Search that survives reflow and resize** | Matches are byte offsets into the document, so the match set is bit-for-bit identical at any width and a highlight follows its text across a rewrap. Most readers lose or shift matches when the window resizes; carrel can't, by construction. The headline feature and the hardest part. |
| **A comfortable measure** | Prose caps at 90 columns and centres, instead of stretching a paragraph across a 200-column terminal. Tables, code and diagrams still use the whole width. |
| **A pager for what your tools print** | `git show \| carrel` reads a diff as a document — a section per file, foldable, searchable. `git config core.pager carrel` and every git command that pages goes through it. |
| **A file-discovery home screen** | Open `carrel` and see what's around you to read, instead of needing a filename. |
| **Clickable links** | Real OSC 8 hyperlinks, with graceful degradation. |
| **Correct emoji and wide characters** | Measured per grapheme cluster, never per codepoint. |
| **Complete markdown** | CommonMark + GFM, footnotes, tables, definition lists, frontmatter, and LaTeX math as terminal box art. Every claim here is [a test](https://github.com/VaHughes/carrel/blob/main/crates/carrel/tests/conformance.rs). |
| **A GUI, eventually** | Planned and designed for, **not yet built.** So that people who don't use terminals can read markdown too. |

## Install

**Installer script** (Linux x86_64/aarch64 — static musl included — and macOS):

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/VaHughes/carrel/releases/latest/download/carrel-installer.sh | sh
```

**Homebrew** (macOS and Linux):

```bash
brew install VaHughes/tap/carrel
```

**Fedora** (43 and 44, x86_64 and aarch64):

```bash
sudo dnf copr enable vahughes/carrel
sudo dnf install carrel
```

**Cargo** (any platform with a Rust toolchain), or prebuilt via
[`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo install carrel        # builds from crates.io
cargo binstall carrel       # fetches the release binary
```

Prebuilt archives for every target are on the
[releases page](https://github.com/VaHughes/carrel/releases). AUR (`carrel`, `carrel-bin`)
is prepared but waiting on Arch: AUR account registration is closed while the Arch team
handles an ongoing supply-chain campaign against the repository. Distribution is a
first-class goal, not an afterthought — the
research was blunt about why: one of the best-featured terminal markdown renderers in
existence has **under a hundred stars**, because nobody can find it. In this niche,
packaging beats code.

**Open `.md` files from your file manager** (optional, Linux): install
[`contrib/carrel.desktop`](https://github.com/VaHughes/carrel/blob/main/contrib/carrel.desktop) (it is also inside every release archive)
and make carrel your markdown handler *if you want it* — carrel never takes the default by
itself:

```bash
cp contrib/carrel.desktop ~/.local/share/applications/
xdg-mime default carrel.desktop text/markdown
```

Carrel is a Unix program. There are no Windows binaries and none are planned — the port was
scoped, costed, and then declined, because a terminal reader nobody here can sit in front of
and read with is not one worth publishing.

## Pipe into it

Carrel is a pager for markdown:

```bash
gh pr view 128 | carrel
git show HEAD:README.md | carrel
an-agent --stream | carrel     # content appears as it arrives
```

`carrel -` forces stdin mode, and `cmd | carrel - pattern` prints a match report — exiting 1
when nothing matched, so `if carrel doc.md pattern; then` behaves like grep. Piping
*out* produces plain text, so `cmd | carrel | grep` behaves; `carrel --render FILE` keeps
weight, slant, strike and OSC 8 links (never a colour) for embedding in another tool's
output, and `carrel --tasks FILE` prints the document's task list as checkbox lines. While a producer is
still writing, the reader is already open — and your position and search matches hold as
content arrives, because positions never depend on the screen. Press `F` to pin the view
to the end while it grows, and `y` to copy the code block you are looking at
(`]` and `[` step between them).

### Diffs, and git's pager

A pipe — or a `.diff` / `.patch` file — is read as a diff when it looks like one: a heading
per commit and per file, hunks as code, additions and removals in your theme's own colours.
Because files become *sections*, everything carrel already does to sections works on a
diff: fold a file away with `za`, collapse the whole changeset with `zM`, jump between
files from the outline, and search across all of it without the results moving when you
resize.

```bash
git show | carrel
git log -p | carrel
git config core.pager carrel     # and then every git command that pages
```

A `.md` file is **never** read as a diff, whatever it contains — so a document *about*
diffs stays a document. `--diff` forces the reading, `--no-diff` refuses it.

## Configuration

Optional. Carrel writes `$XDG_CONFIG_HOME/carrel/config` (or `~/.config/carrel/config`) itself
when you change a setting in the app, and you can edit it by hand. One `key = value` per line
(`place` may repeat); unknown keys and `#` comments are ignored.

| Key | Default | What it does |
|---|---|---|
| `max_width` | `90` | The reading measure: prose wraps at this many columns and centres on the page. Tables, code blocks, images and diagrams ignore it and use the full width. Set `0` to turn it off and let prose fill the terminal. |
| `theme` | `terminal`, or `omarchy` where there is one | Palette name — one of the seventeen listed under the example. `terminal` inherits your terminal's own colours; `omarchy` follows the desktop (see below). `T` cycles them in the app and saves your choice. |
| `hints` | `true` | The lamplight hint row along the bottom. `H` toggles it. |
| `titles` | `false` | Show each document's own title — `title:` from frontmatter, else its first heading — instead of its file name. Falls back to the name for a file that has neither. |
| `outline_margin` | `false` | The section tree pinned in the left margin, current section lit, on terminals wide enough to spare the columns. Click a heading to jump. Off by default because it moves the text column. |
| `breadcrumb` | `true` | The section path pinned atop the page while you scroll — `The Book ▸ Chapter ▸ Detail` — with a rule under it. `B` toggles it. Documents with no headings never show one. |
| `root` | — | The directory the home screen lists. `d` picks one in the app. |
| `place` | — | A remembered favourite root, offered by the directory picker. This key repeats, newest first, capped at eight; choosing a root with `d` records it. |

```ini
# ~/.config/carrel/config
max_width = 72
theme = gruvbox-dark
```

**The palettes**, in the order `T` walks them:

`terminal`, `carrel-dark`, `carrel-light`, `catppuccin-mocha`, `catppuccin-latte`,
`gruvbox-dark`, `gruvbox-light`, `tokyo-night`, `nord`, `dracula`, `solarized-dark`,
`solarized-light`, `everforest`, `rose-pine`, `kanagawa`, `synthwave`, `oceanic`.

`dark` and `light` are accepted as aliases for `carrel-dark` and `carrel-light`, and on a
desktop that publishes a palette `omarchy` rides at the end of the rotation (below). A name
carrel does not know is not an error: it opens on `terminal` and says so in the status bar,
so a config written for a future version still starts.

### On Omarchy, it wears what the desktop is wearing

Omarchy publishes the active theme as a terminal palette at
`~/.local/state/omarchy/current/theme/colors.toml` — the same file its alacritty, btop and
helix themes are generated from. Where that file exists, carrel reads it and derives a palette
from it, and a reader with no `theme` on record opens wearing it. Run `omarchy theme set` and an
open carrel follows within a second; no restart, no keypress.

It is one more entry in the `T` rotation, named `omarchy`, so you can leave it for a fixed
palette whenever you like — and `theme = omarchy` in the config pins it. Nothing is read over the
network and nothing outside that one file is consulted; on a machine without Omarchy the option
simply is not offered and `terminal` remains the default.

## Build from source

Requires Rust 1.95+.

```bash
git clone https://github.com/VaHughes/carrel
cd carrel
cargo test --workspace
cargo run -p carrel -- README.md "search"
```

## Architecture

Two crates, one seam.

```
carrel-core/   document model, search, layout primitives.  NO UI DEPENDENCIES, EVER.
carrel/        the terminal frontend (ratatui).  the only frontend that exists today.
```

A GTK4 + WebKitGTK frontend is intended to sit *alongside* `carrel/`, never inside it. None of it
is written — what exists today is the discipline that keeps it possible: `carrel-core` has no UI
dependency, emits semantic scopes rather than ANSI, and exposes no width-dependent type. Every
project surveyed during research that planned a second frontend for "later" never got one, so
those constraints are treated as load-bearing rather than aspirational.

Everything follows from a single invariant:

> There is exactly one authoritative coordinate space: a byte offset into a flattened, unwrapped
> display text. Screen row, wrap column, and highlight rectangle are *derived* functions of
> `(document, width)` — recomputed on resize, never stored. A search hit recorded at width 80 is
> bit-for-bit the same value at width 40.

Search state therefore cannot be invalidated by reflow, because no search state is ever expressed in
display coordinates. Four independent implementations converge on this — Helix's entire terminal
resize handler updates a `Rect`, and VS Code's find feature has *no* wrap-change handler at all.

The rules that keep the second frontend possible are enforced mechanically:

```bash
./scripts/check-discipline.sh
```

## Roadmap

- [x] Workspace, document model, provenance table
- [x] Search over flattened display text — survives reflow by construction
- [x] Grapheme-correct width measurement and greedy line breaking
- [x] The real reflow layer — two-stage break-unit producer and packer
- [x] The TUI: view state, resize path, paint loop, vim motions, incremental search
- [x] File-discovery home screen with a cached index
- [x] OSC 8 hyperlinks, relative-link traversal with a history stack
- [x] Syntax highlighting (syntect, semantic scopes not colours)
- [x] Images (kitty protocol first, half-block fallback everywhere)
- [x] Themes: 17 palettes, cycled live, persisted — plus the desktop's own on Omarchy
- [x] Help overlay, reading-position resume, `[[wikilinks]]`
- [x] Mouse selection that copies clean text (drag, double-click word, triple-click block)
- [x] Outline navigation, live reload, search inside every file
- [x] Mermaid diagrams as Unicode box art
- [x] Frontmatter cards, definition lists, LaTeX math as box art, a conformance suite
- [x] The reading desk begins: a 90-column measure with centred prose, a time-remaining
      estimate, and a home screen you can click
- [x] stdin/pager mode — pipe in, stream as it arrives, keep your place
- [x] A sticky heading breadcrumb: the enclosing sections, pinned while you scroll
- [x] Section folding — `za`/`zM`/`zR` and click-a-heading; search always unfolds its target
- [x] Diffs read as documents — `git show | carrel`, foldable per file, and git's pager
- [x] Follow mode for a growing pipe; copy a code block with `y`
- [x] Continue reading, bookmarks, backlinks, the outline in the margin
- [x] Packaging: Homebrew, Fedora COPR, the shell installer, crates.io
- [ ] Marginalia — highlights and notes made while reading, stored in the state directory
      and never in the document; anchored on byte offsets, so they survive a resize by
      construction, re-finding themselves after an edit by the quoted text; walked like
      bookmarks, reviewed from an overlay, exported as quote-and-note markdown
- [x] Focus dimming (`S`) — everything outside the paragraph at the centre of the view
      falls into shadow; the quiet place, made literal
- [x] `<details>`/`<summary>` folds natively — the summary becomes a fold point, reusing
      section folding wholesale
- [ ] Search results read as a document — a section per file with context lines,
      searchable again, on the same machinery that reads a diff
- [ ] Image lightbox — Enter opens an image full-screen, kitty protocol first and
      half-block fallback everywhere; `[`/`]` walks the images of the document
- [x] Fuzzy matching for the home filter and the outline picker — best alignment wins,
      ranked; substring no longer
- [ ] Word-level colour inside changed diff lines, so a prose review reads as prose
- [x] Task-list awareness without editing — task-jumping in the reader (`X`), a
      `--tasks` report, the count on the info card; ticking a box is editor creep.
      Home-screen progress glyphs stay out: counting honestly means reading whole files.
- [ ] Tags — frontmatter `tags:` indexed lazily the way titles are, tag-filtered views
- [x] A bookmark list overlay (`"`) — every mark with its context line, Enter jumps,
      Ctrl-O comes back
- [x] Forward links (`l`) — what this note points at, the mirror of backlinks `L`
- [ ] Wide-table horizontal scrolling — cards and wrapping both lose past some width
- [x] Footnote jump-and-return — `%`, to the definition and back
- [x] `carrel --render` — styled ANSI output even when piped: weight, slant, strike and
      OSC 8 hyperlinks, never a colour, `NO_COLOR` reduces it to `--plain`; still never
      fetching anything itself
- [x] Document info card (`I`; `g` belongs to the gg prefix) — words, minutes, structure,
      links, when it last changed
- [x] Places — favourite roots remembered by the picker, newest first, capped at eight;
      choosing a directory records it
- [x] A home list that keeps up — the tree is walked again while the list is on screen, so
      a file written elsewhere appears without a restart; and `d` opens the picker on the
      directory you are already in, so a typed path continues from there
- [ ] Hyphenation at narrow measures — pattern-based breaks below roughly 70 columns
- [x] Auto-read mode (`A`) — the view drifts down a row every 300 ms; any deliberate
      motion takes the wheel back, and the end of the document stops it gently
- [ ] Packaging, remaining: AUR (blocked on Arch), nixpkgs, `.deb`
- [ ] The GUI: GTK4 shell + WebKitGTK content view — **not started, no date**

## Contributing

See [CONTRIBUTING.md](https://github.com/VaHughes/carrel/blob/main/CONTRIBUTING.md). The short version: contributions are welcome inside the
one-coordinate-space invariant above, and `./scripts/check-discipline.sh` is the referee.

## License

MIT OR Apache-2.0, at your option.
