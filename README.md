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

<img src="https://raw.githubusercontent.com/VaHughes/carrel/main/assets/demo.gif" alt="Carrel listing the markdown files in a folder, filtering to README.md, and scrolling through it" width="800">

</div>

> **Status: early, but it runs.** `carrel` shows you what is around you to read; `carrel FILE` opens
> a reader you can click around in, with a search that keeps its place when the window
> resizes. Syntax highlighting, tables, images and mermaid diagrams all render, and the vim
> keys are there for anyone who already knows them. See [Roadmap](#roadmap).

### It never fetches anything

Carrel reads only the folder you point it at: the one named on the command line, else the
`root` you last chose, else the folder you are standing in. Anything wider is a root you
choose yourself — `d` opens a folder browser on the folder you ran `carrel` in, offering
parent, itself, children and your favourites, narrowed by typing; a filter with a `/` in it
completes a path against the filesystem instead. It
sends nothing anywhere — remote images in documents are never fetched; they render as their alt
text. The index it caches lives under `$XDG_CACHE_HOME/carrel` and holds file paths and
modification times, nothing else.

That first sentence is **enforced, not merely intended**. A markdown file is untrusted input — a
shared vault, a downloaded README, anything you did not write — and a link in one can name any
path on the machine. Links resolving inside your folder follow as they always have; one
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
| **Search that survives a resize** | A match is a place in the text, not a place on the screen, so the same matches exist at any width and a highlight follows its text when the lines rewrap. Most readers lose or shift matches when the window resizes; carrel can't, by construction. The headline feature and the hardest part. |
| **Everything worth doing is clickable** | Links, headings, collapse markers, the outline, the panes, the hint row along the bottom — click them. Right-click for a menu of what is under the pointer, or press `≡` on the status row for everything else. Carrel is built for people whose way into the terminal was an AI coding agent and who now have a `PLAN.md` to read; they should not have to learn `zR` first. Every key still works, and `--no-mouse` hands the pointer back. |
| **A comfortable line length** | Prose caps at 90 columns and centres, instead of stretching a paragraph across a 200-column terminal. Tables, code and diagrams still use the whole width. |
| **A pager for what your tools print** | `git show \| carrel` reads a diff as a document — a section per file, collapsible, searchable. `git config core.pager carrel` and every git command that pages goes through it. |
| **A file-discovery home screen** | Open `carrel` and see what's around you to read, instead of needing a filename — and walk the tree from the path row above the list, a segment at a time. |
| **Clickable links** | Click a link, wherever it is painted: a markdown file beside it opens in the reader, and a URL is copied to your clipboard to paste where you want it. In a terminal that supports it, a link is a real hyperlink too; elsewhere it is plain text. Carrel never fetches a URL and never launches a program to open one. |
| **Correct emoji and wide characters** | Width is measured per visible character, not per code point, so emoji and CJK text line up instead of drifting. |
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
cargo install carrel --locked   # builds from crates.io, with the tested dependency set
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
content arrives, because positions never depend on the screen. Press `F` to keep up with
the end while it grows, and `y` to copy the code block you are looking at
(`]` and `[` step between them).

### Diffs, and git's pager

A pipe — or a `.diff` / `.patch` file — is read as a diff when it looks like one: a heading
per commit and per file, hunks as code, additions and removals in your theme's own colours.
Changed words and punctuation stand out within replacement lines, so prose edits are
easy to spot; under `NO_COLOR`, they remain underlined.
Because files become *sections*, everything carrel already does to sections works on a
diff: collapse a file with `za`, collapse the whole changeset with `zM`, jump between
files from the outline, and search across all of it without the results moving when you
resize.

```bash
git show | carrel
git log -p | carrel
git config core.pager carrel     # and then every git command that pages
```

A `.md` file is **never** read as a diff, whatever it contains — so a document *about*
diffs stays a document. `--diff` forces the reading, `--no-diff` refuses it.

### Wide tables

Wide tables open as cards. Press `t` or choose **Cards ↔ columns** from the
right-click menu to keep their columns aligned. Use `Shift-←` / `Shift-→` to
scroll the first visible wide table, or click its `[←]` / `[→]` buttons below
it. Search automatically brings a match's column into view.


## Using the mouse

Carrel captures the mouse, so clicks reach it rather than your terminal. What that buys:

| | |
|---|---|
| **The path row** | The folder you are in, under the banner, spelled as its own segments — `~ / Work / carrel / docs`. Click any one of them to go there; the `↑` at its head goes up one folder, and `Backspace` does the same from the keyboard. |
| **The `⌂`** | At the left of the reader's status row: back to the file list, rooted at the document's own folder when there is no list behind it to return to. |
| **Right-click** | A menu for whatever is under the pointer — collapse this section, copy this code block, open or copy this link, cards or columns for this table. Right-click anywhere else, or click the **`≡`** at the end of the status row, and you get the global menu instead. Every row shows the key that does the same thing. |
| **A link** | Click it. A markdown file beside it opens in the reader; a URL is copied to your clipboard. |
| **A heading, or a `▸` / `▾` in the margin** | Collapses or expands that section, or that `<details>` block. |
| **The hint row along the bottom** | Every hint is a button and looks like one — `↑/↓ scroll`, `/ search`, `o outline`, `h help`, each a chip on the status bar's surface. So are `T theme` and `q quit` on the status row, and the lamp at the far left, which hides the hint row itself. |
| **A row in a pane** | The outline, the bookmark list and both link panes open the row under the pointer. |
| **The margin outline** | Click a section to jump to it. |
| **Text** | Drag to select; release copies it. Double-click takes the word, triple-click the whole block — which is how you copy a code block cleanly, with no gutter and no wrapping. |
| **The scrollbar** | Drag the thumb, or click the track to page toward the pointer. The wheel scrolls, and gathers speed if you keep spinning it. |
| **Hovering** | Whatever the pointer is over lights up, if clicking it would do something. Decoration only — a click always resolves from where it landed. |

The trade is that your terminal's own text selection stops working while carrel has the
pointer. Most terminals let you **hold Shift** to bypass that and select as usual; if
yours doesn't, or you would rather not, `--no-mouse` (or `mouse = false` in the config)
gives the pointer back for good. Nothing becomes unreachable — every action has a key.

### Images, highlights, and notes

Click an image, press Enter at one when no link is selected, or choose **Images…**
from the menu to open the lightbox. `[` / `]` or the arrow keys walk the document’s
images; Esc, Enter, or `q` closes it and keeps your reading position. Local images
fit the window using the available graphics protocol or colored blocks. Remote
images are never fetched.

Select text and press `v` to highlight it, or `a` to add a note. With no selection,
these use the paragraph at your reading position. The right-click menu offers both.
In the note editor, Enter or Ctrl-S saves, Ctrl-J adds a newline, and Esc cancels.
Pasted text stays text, including newlines.

`V` opens **Notes and highlights**; select a row to jump, edit, delete, or export.
`M` walks attached notes and highlights. Notes live under
`$XDG_STATE_HOME/carrel/marginalia/` (normally `~/.local/state/carrel/marginalia/`),
never in the source document. They survive resizing and use the saved quote and
surrounding text to find their place after an edit. Missing or ambiguous quotes
remain available as unresolved notes. **Export** (`E` in the list) writes a
quote-and-note Markdown file alongside the saved notes and reports its path.
For piped documents, notes last for the session and export through the terminal
clipboard (OSC 52).

## Configuration

Optional. Carrel writes `$XDG_CONFIG_HOME/carrel/config` (or `~/.config/carrel/config`) itself
when you change a setting in the app, and you can edit it by hand. One `key = value` per line
(`place` may repeat); unknown keys and `#` comments are ignored.

| Key | Default | What it does |
|---|---|---|
| `max_width` | `90` | The line length: prose wraps at this many columns and centres on the page. Tables, code blocks, images and diagrams ignore it and use the full width. Set `0` to turn it off and let prose fill the terminal. |
| `theme` | `terminal`, or `omarchy` where there is one | Palette name — one of the seventeen listed under the example. `terminal` inherits your terminal's own colours; `omarchy` follows the desktop (see below). `T` steps to the next one in the app and saves your choice. |
| `hints` | `true` | The hint row along the bottom. `H` toggles it. |
| `titles` | `false` | Show each document's own title — `title:` from frontmatter, else its first heading — instead of its file name. Falls back to the name for a file that has neither. |
| `outline_margin` | `false` | The section tree pinned in the left margin, current section lit, on terminals wide enough to spare the columns. Click a heading to jump. Off by default because it moves the text column. |
| `breadcrumb` | `true` | The heading bar: the section path pinned atop the page while you scroll — `The Book ▸ Chapter ▸ Detail` — with a rule under it. `B` toggles it. Documents with no headings never show one. |
| `mouse` | `true` | Capture the mouse, so clicks reach carrel rather than the terminal. Set `false` — or pass `--no-mouse` for one run — to hand the pointer back, and your terminal's own selection, scrollback and context menu work as they do anywhere else. Every action stays reachable from the keyboard either way. |
| `root` | — | The folder the home screen lists. `d` picks one in the app. |
| `place` | — | A favourite root, starred in the folder browser `d` opens. This key repeats, newest first, capped at eight; choosing a root with `d` records it. |

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

The gates CI runs, the testing tiers, and the conventions are in
[`docs/DEVELOPMENT.md`](https://github.com/VaHughes/carrel/blob/main/docs/DEVELOPMENT.md).

### Running your own build as `carrel`

While working on Carrel you want the `carrel` command to be *your* build, not the released one
a package manager installed. Symlink it rather than copying — a copy is a snapshot that goes
stale the moment you rebuild, and a stale binary that still answers to `carrel` will have you
debugging a fix you already made:

```bash
cargo build                                                    # debug, ~2s incremental
ln -sfn "$PWD/target/debug/carrel" ~/.local/bin/carrel         # once
```

From then on `cargo build` *is* the install step, and `carrel --version` reports the workspace
version. Two things to know:

- **`cargo clean` breaks the link** until the next `cargo build`, and so does a build that fails
  — the symlink keeps pointing at a file that is gone. `carrel: command not found` after a clean
  means exactly this, not a broken PATH.
- **The debug build is not the shipping build.** It is roughly twenty times the size and starts
  noticeably slower on a large tree of files; syntax highlighting and the initial walk are where you
  will feel it. Point the link at `target/release/carrel` and `cargo build --release` if you are
  judging performance rather than behaviour.

To go back to the packaged binary, remove the symlink and reinstall by whichever route
[Install](#install) describes for your system.

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

Clicks go through the same seam. Keys and pointer events both become values of one shared
`Action` enum before anything downstream sees them, so the state machine has never heard of
either — which is what lets a second frontend produce the same intents from menu items and
`GtkEventControllerKey`. For chrome whose position depends on what is being drawn (the hint
row elides right to left as the terminal narrows), the paint pass **records** where it put
each thing and the event loop reads that back, rather than a hit-test re-deriving geometry
the painter already computed. Re-derivation is how a click ends up one cell off its glyph,
which no frame test can see.

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

The module map, the decisions already made, and the pinned dependencies are in
[`docs/ARCHITECTURE.md`](https://github.com/VaHughes/carrel/blob/main/docs/ARCHITECTURE.md).

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
- [x] Themes: 17 palettes, switched live, persisted — plus the desktop's own on Omarchy
- [x] Help overlay, reading-position resume, `[[wikilinks]]`
- [x] Mouse selection that copies clean text (drag, double-click word, triple-click block)
- [x] Outline navigation, live reload, search inside every file
- [x] Mermaid diagrams as Unicode box art
- [x] Frontmatter cards, definition lists, LaTeX math as box art, a conformance suite
- [x] The reading desk begins: a 90-column measure with centred prose, a time-remaining
      estimate, and a home screen you can click
- [x] stdin/pager mode — pipe in, read as it arrives, keep your place
- [x] A heading bar: the enclosing sections, pinned while you scroll
- [x] Collapsible sections — `za`/`zM`/`zR` and click-a-heading; search always expands its target
- [x] Diffs read as documents — `git show | carrel`, collapsible per file, and git's pager
- [x] Keep up with the end of a growing pipe; copy a code block with `y`
- [x] Continue reading, bookmarks, backlinks, the outline in the margin
- [x] Packaging: Homebrew, Fedora COPR, the shell installer, crates.io
- [x] Click-first: links, collapse markers, pane rows and the whole hint row are buttons,
      on a click-target registry the paint pass fills; `--no-mouse` gives the pointer back
- [x] Menus: right-click for what is under the pointer, `≡` for everything else, each row
      printing the key that does the same thing — every action carrel has is now reachable
      without knowing one, and a test says so exhaustively
- [x] Hover, and one first-run line — the click-first pivot, finished
- [x] Marginalia — highlights and notes made while reading, stored in the state directory
      and never in the document; anchored on byte offsets, so they survive a resize by
      construction, re-finding themselves after an edit by the quoted text; walked like
      bookmarks, reviewed from an overlay, exported as quote-and-note markdown
- [x] Focus dimming (`S`) — everything outside the paragraph at the centre of the view
      falls into shadow; the quiet place, made literal
- [x] `<details>`/`<summary>` collapses natively — the summary becomes a collapse point,
      reusing section collapsing wholesale
- [x] Search results read as a document — `Tab` in home-screen search opens the
      hits as generated markdown, a section per file with a link per match, so the
      outline, collapsing and a second search all work on the results, and every match
      jumps to its line
- [x] Image lightbox — click an image or press Enter at one to view it full-screen;
      `[`/`]` walks the images, using the available terminal protocol or colored blocks
- [x] Fuzzy matching for the home filter and the outline picker — best alignment wins,
      ranked; substring no longer
- [x] Word-level colour inside changed diff lines, so a prose review reads as prose
- [x] Task-list awareness without editing — task-jumping in the reader (`X`), a
      `--tasks` report, the count on the info card; ticking a box is editor creep.
      Home-screen progress glyphs stay out: counting honestly means reading whole files.
- [ ] Tags — frontmatter `tags:` indexed lazily the way titles are, tag-filtered views
- [x] A bookmark list overlay (`"`) — every bookmark with its context line, Enter jumps,
      Ctrl-O comes back
- [x] Forward links (`l`) — what this note points at, the mirror of backlinks `L`
- [x] Wide-table horizontal scrolling — `t` selects columns; `Shift-←` / `Shift-→`
      or the table’s `[←]` / `[→]` buttons pan across the full row; search reveals its column
- [x] Footnote jump-and-return — `%`, to the footnote text and back
- [x] `carrel --render` — styled ANSI output even when piped: weight, slant, strike and
      OSC 8 hyperlinks, never a colour, `NO_COLOR` reduces it to `--plain`; still never
      fetching anything itself
- [x] Document info card (`I`; `g` belongs to the gg prefix) — words, minutes, structure,
      links, when it last changed
- [x] Favourites — roots remembered by the folder browser, newest first, capped at eight;
      choosing a folder records it, and the browser always lists them, starred
- [x] A home list that keeps up — the tree is walked again while the list is on screen, so
      a file written elsewhere appears without a restart; and `d` browses from the folder
      you ran `carrel` in, highlight parked on here, so enter alone reads where you are,
      typing filters, and `Tab` goes in
- [ ] Hyphenation at narrow measures — pattern-based breaks below roughly 70 columns
- [x] Auto-read mode (`A`) — the view scrolls slowly on its own, a line every 300 ms; any deliberate
      motion takes the wheel back, and the end of the document stops it gently
- [ ] Packaging, remaining: AUR (blocked on Arch), nixpkgs, `.deb`
- [ ] The GUI: GTK4 shell + WebKitGTK content view — **not started, no date**

## Contributing

See [CONTRIBUTING.md](https://github.com/VaHughes/carrel/blob/main/CONTRIBUTING.md). The short version: contributions are welcome inside the
one-coordinate-space invariant above, and `./scripts/check-discipline.sh` is the referee.

## License

MIT OR Apache-2.0, at your option.
