# Changelog

Versions are calendar dates, `YYYY.M.D` (Eastern time).

## 2026.9.2

The first step of the click-first pivot: carrel is meant for people who arrived
at the terminal because an AI agent lives there and now need to read what it
wrote. They click. So the things that look clickable are, and the machinery
every later clickable surface will need is in place underneath.

### Things that behave differently

- **Click a link.** Anywhere it is painted, selected or not — no need to select
  it with `Tab` first. A markdown file beside it opens in the reader, exactly as
  `Enter` has always opened it. A URL is **copied to your clipboard**, with a
  note saying so, to paste where you meant it to go; the `l` pane does the same
  for an external destination, which it used to only name.
  **Carrel does not launch a browser**, and it still fetches nothing. Opening a
  URL would mean spawning a program on behalf of a document carrel did not
  write — usually one an agent assembled out of pages nobody read — and then
  deciding, for every scheme anyone invents, whether that program should see it.
  Copying decides nothing. Carrel still depends on no HTTP client and no TLS
  library, and `cargo tree` is how you check that rather than taking our word
  for it. (Local links that leave your library still ask for a second Enter,
  exactly as they did in 2026.9.1.)
- **The fold markers are buttons.** The `▸` / `▾` in the left margin folds and
  unfolds what a click on the heading beside it folds. It always looked like a
  button; now it is one.
- **A click in a pane opens the row under the pointer** — the outline, the
  bookmark list, and both link panes — rather than the row the keyboard cursor
  happened to be resting on. And an open pane now takes every click inside it:
  clicking one used to start a text selection in the document behind it.
- **`--no-mouse`, and `mouse = false` in the config file.** Carrel captures the
  pointer by default, which is what stops your terminal's own selection and
  context menu from working. This hands it back. Every action stays reachable
  from the keyboard, and the flag wins over the config key for one run.

- **The footer is a row of buttons.** Every hint along the bottom — `spc page`,
  `/ search`, `o outline`, `h more`, and the rest, whichever set the current
  state is showing — does its thing when you click it. So do `T theme` and
  `q quit` in the status row, which have been painted as words for a while on
  the grounds that a key nobody can see is a feature nobody has. The lamp was
  already clickable and still is; it just stopped being a hardcoded rule about
  the bottom-left three cells and became a painted thing that registers like
  every other.
  A hint's click does what its **first** key does, so `j/k` scrolls down and
  `n/N` goes to the next match. A row that names no key — `type` — is a
  sentence, not a button, and stays inert. A test presses each hint's key
  through the real dispatcher and asserts it produces exactly the action
  clicking it would: the footer cannot advertise one thing to the keyboard and
  do another to the mouse.

- **Menus, and an `≡` to open one.** **Right-click in the document** and you
  get a menu for whatever is under the pointer: a heading offers to fold its
  section, a link offers to open or copy it and to show what points at it, a
  code block offers to copy itself, a table offers the card view. It is
  positioned before it opens, so its rows act on what you clicked rather than
  on wherever the view happens to be. **Right-click anywhere else** — the
  status row, the hint footer, the breadcrumb, the margin, the scrollbar, the
  home screen — and you get the global menu: document info, spotlight,
  auto-read, themes, the key hints, the breadcrumb, help, and the way out.
  The same global menu opens from the **`≡` at the right end of the status
  row**, with an ordinary left click, because nobody should have to guess that
  right-clicking does anything.
  Inside a menu, `↑`/`↓` move, `Enter` chooses, `Esc` or `q` closes, and a
  click outside closes without choosing. Moving the pointer over a row lights
  it. Nothing is lit when a menu opens. **Every row shows the key that does
  the same thing** — which is how a clicking reader becomes a keyboard reader
  six months later without ever being told to — and those keys come from the
  keymap itself, so a menu cannot advertise a shortcut carrel does not bind.
  A greyed row says what carrel could do here and declines: `Follow the end`
  on a document that is not growing, `Back` with nothing behind you.
- **`Back` has a way in that is not a chord.** `Ctrl-O` went back after
  following a link, and it is not a key anyone guesses. It is the first row of
  the global menu now.
- **Copy a local link.** Clicking a link to a file beside it opens it; the
  menu's `Copy link` puts its path on the clipboard instead. A URL is still
  copied and never opened.
- **What the pointer is over lights up.** A link, a footer button, `T theme`,
  the `≡`, a row of an open pane or of the home list — if a click would do
  something there, moving the pointer over it says so. It is decoration and
  never a decision: every click resolves from its own coordinates, so a
  terminal that does not report pointer motion loses the highlight and nothing
  else.
- **The path above the file list is a row of buttons.** The home screen has
  always shown which directory it is listing; now each segment of it goes
  there — `~ / Work / carrel / docs`, click `Work` and you are in `Work`. The
  `↑` at the head of the row goes up one level, and `Backspace` does the same
  from the keyboard. On a narrow terminal the row drops whole segments from
  the shallow end and marks the cut with `…`, because the end you are in is
  the end you navigate from. Walking the tree this way deliberately does
  **not** change where `carrel` opens next time — `d` is still how a
  directory becomes your default.
- **A `⌂` on the reader's status row goes back to the file list.** With a home
  screen behind the document that is what `q` already did; for a document
  opened directly, or piped in, `q` quits — so the icon opens a list rooted at
  the document's own directory instead of ending the program. Both status-row
  icons stand down on a terminal too narrow to keep the reading percentage,
  since each has another way in.
- **One line, on the first run, and then never again.** A launch that has
  never remembered a reading position says `click anything · right-click for a
  menu` where the key hints go. Doing anything at all retires it — and reading
  something is what makes the next launch quiet.

### Four things that were quietly wrong

Clicking exposed these; they were harmless only because so little was
clickable.

- **A click on the far right of the status row paged the document.** The
  scrollbar claimed its whole column, top to bottom, instead of the track's own
  rows.
- **A click in the margin outline's columns, below the text, jumped to a
  heading.** Same shape of bug: bounded at the top and not at the bottom. It
  needed a document with more headings than the terminal is tall to show up at
  all, which is why it survived.
- **An open pane ignored `j` and `k` when you had come from the home screen.**
  The two event loops disagreed about which keymap owns a keystroke: opening a
  file directly gave the backlinks, forward-links and bookmark panes their own
  keys, opening the same file from the home screen gave them the reader's, so
  `j` scrolled the document *underneath* the open pane. Both loops now ask one
  function.
- **A double-click needed both presses on exactly the same cell.** A hand
  drifts, and the drift turned it into two single clicks. One cell of slack now.

### Under it

- A click-target registry: the painter records where it put a thing, and the
  event loop reads that back, instead of a hit-test re-deriving geometry the
  painter already computed. Continuous surfaces — the text body, the scrollbar
  thumb — keep the geometry-function-and-its-inverse they already had, which is
  right for them. One `TestBackend` round-trip now guards every registered
  target at once, and every clickable surface added later inherits it.
- Moving the pointer no longer repaints the whole screen once per cell crossed.
  Mouse capture asks the terminal to report every motion, and carrel was
  drawing a full frame, plus its two synchronized-update escapes, for each one.
- **A link underneath an open pane used to be drawn back on top of it.**
  Terminal hyperlinks are re-emitted after each frame from the coordinates the
  link was painted at, not from what is on screen — so any panel covering a
  link put its text back over the panel, in the panel's own colours. Every
  overlay had this; menus open on top of prose every time, which is how it was
  finally noticed.
- **Links were painted in the wrong colour in 15 of the 17 themes.** Terminal
  hyperlinks are re-emitted after each frame by repainting the link's own
  cells, and that repaint hard-coded carrel's amber — so every visible link
  was painted in the theme's colour and then immediately painted back in
  amber. It has been that way since hyperlinks shipped. The repaint reads the
  finished frame now instead of guessing, which is also what lets a hovered
  link stay hovered.
- **The fold markers were printed wrong in the man page.** `▸` and `▾` rendered
  as the literal text `5b8` and `5be` in every `man carrel` since they were
  documented — a two-character escape given a four-character name. `man` hides
  the warning that says so by default. Fixed, and guarded.
- Changing the theme now travels the same route as everything else instead of
  being intercepted by each event loop before the state machine saw it. That
  was invisible until a menu row needed to do it — and would have done nothing.

## 2026.9.1

An adversarial audit of the whole tree — every module read line by line, then
attacked with hostile documents, malformed input and a real pty. Fifteen
changes; every fix carries a regression test that was watched failing first.
Alongside them, one maintainer report the audit did not find — what the
directory picker means by "here" — and the documentation sweep that followed it.

### Things that behave differently

Five changes you may notice, listed first because they are the ones that could
surprise you:

- **A link that leaves your library asks before it opens.** Carrel is rooted at
  the home screen's root, the opened document's own directory, or the working
  directory for a pipe. Links inside it follow exactly as before; one resolving
  outside names the path and waits for a second Enter. See below for why.
- **`carrel FILE PATTERN` exits 1 when nothing matched**, as grep, rg and ag
  all do. If you scripted around the old always-zero exit, that changes.
- **An unknown or misplaced option is now an error** instead of being read as a
  filename or a search pattern, and a `[W]` argument that is not a number says
  so instead of silently using 80. `--` ends the options, so a file whose name
  begins with a dash is openable for the first time.
- **`d` opens on the directory you ran `carrel` in.** It used to open on the
  directory the home screen was showing, with your remembered places leading
  the list — so with a saved `root = …` on file, the first thing the picker
  offered was the last place you read in, not the one you had just `cd`-ed to,
  and enter went there. The working directory now leads its own subdirectories,
  so enter alone reads where you are. Remembered places lead the *empty* menu:
  one `Esc` clears the input and brings them back, as it always has.
- **The minimum supported Rust is 1.95**, not the 1.90 three files claimed. The
  floor comes from `merman`, a dependency pinned exactly; 1.90 could never have
  resolved, so a `cargo install` on it failed for the user rather than for us.
  CI now builds on 1.95 so the claim stays honest.

### Fixed

- **Auto-read never worked, and pinned a CPU core when you pressed it.** The
  `A` tick was nested inside the debounced-resize branch, so it fired only
  while a window was being dragged: the page never drifted, and because the
  clock therefore never advanced the event loop spun at 100% of a core until
  you noticed the fan. Measured at 399 CPU ticks per four seconds against 0 for
  any other key. The unit tests could not see it — they call the action
  directly, which is the one caller that worked — so the guard is now a pty
  test that presses `A` and waits with no resize in sight.

- **Pressing `y` after following a link could abort the reader.** The block
  cursor is an index into the document it was set against, and neither opening
  a document nor reloading one cleared it, so a copy after following a link to
  a shorter file indexed a block that no longer existed. The selection had the
  same shape and was already half-fixed: reload cleared it with a comment
  saying why, and open never learned. Both now go through one function, which
  is what stops them drifting apart a third time.

- **Any markdown file could read any file on the system.** `[x](/etc/passwd)`
  rendered it — no `..` required, because joining an absolute path discards the
  base entirely. The README says in bold that carrel reads only the directory
  you point it at; the sending half of that was true and well defended, and the
  reading half was not. A markdown file is untrusted input: a shared vault, a
  downloaded README, anything you did not write yourself. Wikilinks went
  through the same hole. Both are contained now, and both canonicalize, so a
  symlink pointing out of the tree is caught too.

- **Large documents open in a fraction of the time.** Two separate quadratics,
  both on the path you reach by opening a file. The chunking that exists to
  *bound* work on a huge paragraph was itself recomputing its boundaries once
  per chunk, so a block of k chunks paid k+1 full scans of it — and that is the
  resize path, walked twice per relayout. And a line with no ASCII whitespace
  (a CJK paragraph with embedded ASCII, a pasted minified or base64 run) made
  the script scanner rescan to the start of the line for every candidate.

  |            | before | after |
  |---|---|---|
  | 8 MB single paragraph | 6361 ms | 138 ms |
  | 20 MB single paragraph | never finished | 321 ms |
  | 256 KB whitespace-free line | 8631 ms | 11 ms |

- **The terminal comes back however carrel exits.** A default-disposition
  signal ran neither the guard nor the panic hook, so `pkill carrel`, a session
  manager at logout, or killing a wedged reader left the alternate screen up,
  mouse capture on and the cursor hidden — needing `reset`. All four of
  SIGTERM, SIGHUP, SIGINT and SIGQUIT restore fully now. A panic mid-frame also
  left synchronized-update mode set, so the shell prompt might never appear.
  And `carrel doc.md | head` panicked with a Rust backtrace note: Rust ignores
  SIGPIPE, so a closed pipe was a panic rather than the ordinary end of a
  filter. It exits 0 now, like every other Unix tool of its shape.

- **Ctrl-C in the search prompt typed a `c` into the query.** It was the only
  one of seven key dispatchers without the binding, and it never inspected the
  modifier. Raw mode clears ISIG, so no SIGINT was generated either and the
  reflex did nothing at all.

- **A link to a FIFO wedged the reader forever.** The size guard measured with
  `metadata().len()`, which is zero for a FIFO, `/dev/zero` and most of
  `/proc`, so the guard passed and the unbounded read behind it ran anyway —
  with the terminal already in raw mode and the event loop stalled, so no
  keystroke could reach it. `/dev/zero` read until the OOM killer arrived.

- **Clicking a wide table selected the wrong text.** Paint put tables, code,
  math and images in the bleed column and re-centred a wide table; the
  hit-test bounded every click by the prose column. They agreed for prose and
  diverged for everything else at 95 columns or wider — any maximized
  terminal — so a click landed thirteen columns from the pointer and the outer
  thirds were rejected outright. Geometry lives in one function now.

- **`--plain` and `--render` passed a document's escape bytes to your
  terminal.** `plain.rs` promised "never an escape byte" and then copied
  document text straight out, so a markdown file you merely read could set the
  terminal title of whatever it was piped into. The TUI was safe only
  incidentally. An entity-encoded `&#27;` in a link destination could also
  smuggle an escape into `--render`'s OSC 8 hyperlinks. Both are stripped now,
  at one shared point each, and `NO_COLOR --render` really is byte-identical
  to `--plain` as documented — it had been dropping blockquote bars.

- **State on disk survives a crash and a second reader.** All four files were
  read-modify-write through a call that truncates in place, so two carrels
  interleaving lost one's whole session and a crash mid-write destroyed the
  file. They are written to a temp sibling and renamed now. A path containing
  a TAB also corrupted the bookmarks file permanently, appending one line per
  save forever, and the comment arguing that could not happen had the argument
  backwards.

- **An ignore file above your root no longer empties the home screen.** The
  walk read `.gitignore` in every ancestor directory and `core.excludesFile`
  from your global git config, so a `*.md` line anywhere above the root
  silently showed you nothing, with no note and no way to find out why.

- **`X` walked tasks out of order**, skipping the first and looping wrong in
  both directions — its test asserted only that three distinct tasks appeared,
  which the broken walk satisfied.

- **`math_art` was quadratic per row and cubic on nested fractions**, laid out
  for every equation on the UI thread on open and resize, and its width could
  silently cap while the content kept growing. Nested fractions at depth 2,000
  took 1.63 s and are now refused in microseconds, falling through to the
  literal-source rendering that already existed for maths that will not parse.

- **The release workflow published without running a single test.** CI triggers
  on pushes to a branch, which a tag push does not match, so `dist` built and
  uploaded six target archives with nothing having checked them. The packaging
  recipes at the tag also built the *previous* release — one of them five
  releases stale — because the version stamp landed in a commit after the tag,
  and the guard compared the recipes only to each other.

- **Documentation that was not true.** The README's only config example named a
  theme that does not exist, and the seventeen real palette names appeared in
  no user-facing document at all. `--help` said `q` quits when it closes the
  file, and never mentioned `Q`. The `.desktop` entry passed multiple files, so
  selecting two markdown files in a file manager searched the first for the
  second's filename and vanished. The man page was missing three config keys
  and the entire mouse section that a test exempted it from carrying.

- **Bounded what was not.** The navigation trail, the OSC 52 clipboard write,
  the index cache directory and the bookmarks file all grew without a limit in
  a project that caps everything else and says why. The index cache was also
  named by a hash std documents as unstable across releases, so a toolchain
  upgrade silently orphaned every one of them.

### Also

- **On Omarchy, carrel wears what the desktop is wearing.** The `terminal` theme inherited
  your terminal's background and foreground, but every accent — headings, links, the search
  lamp, the syntax colours — was carrel's own house green and amber whatever theme the rest
  of the desktop had on. On a purple desktop that read as a window that had not been told.
  Omarchy publishes the active theme as a terminal palette at
  `~/.local/state/omarchy/current/theme/colors.toml`, the same file its alacritty, btop and
  helix themes are generated from; carrel now derives a palette from it, offers it as
  `omarchy` in the `T` rotation, and opens wearing it when no `theme` is on record. It is
  re-read once a second, so `omarchy theme set` restyles a reader that is already open —
  no restart, no keypress. Everything derived is blended toward the page or toward the ink
  rather than toward black or white, so a light Omarchy theme comes out light; and a slot
  whose colour would be illegible against that theme's own background falls through to one
  that is not, because an invisible link is not a link. Nothing outside that one file is
  consulted, and on a machine without Omarchy the option is not offered.

- **A spun wheel scrolls faster.** Every wheel notch moved exactly three lines, so crossing
  a long README meant a great many of them. Notches arriving in quick succession now
  compound, three lines at a time up to twelve; a pause, or a change of direction, drops
  straight back to three so a correction stays precise. The home screen is deliberately not
  accelerated — the wheel moves the selection there, and a selection that gathers speed
  overshoots the row you were aiming for.

- **The search counts matches while you are still typing.** The hits were found and
  highlighted on every keystroke, but the status bar kept showing the scroll percentage
  until you pressed Enter, so the one question you have while typing a needle — is this
  finding anything — went unanswered until you had committed to it. It now reads
  `12 matches` as you type, and `no matches` when there are none. `3 of 12` still takes the
  slot back the moment you accept one.

- **The manual documents the home screen, and its bookmark-list key appears at last.** `"`
  was written into `carrel.1` as a bare `.B "`, which troff reads as the start of a quoted
  argument: the description painted with no key beside it, for as long as the key has
  existed. The home-screen section listed four of its fifteen keys — movement, `Enter`, the
  `1`/`2`/`3` resume rows, `T`, `H`, `h`/`F1`, `q` and `Esc` were all missing. `?`, `Home`
  and `End` were bound and undocumented, and `Ctrl-E`/`Ctrl-Y` were described as scrolling
  "without moving the reading position", which is not a thing carrel has. `--help` gained
  the link keys (`Tab`, `Shift-Tab`, `Enter`), `Esc`, `H`, `B`, `PgDn`/`PgUp`, the `-h`/`-V`
  short forms and `carrel --render -`. Undocumented until now: `max_width` raises anything
  under 20 to 20, and a key repeated in the config takes its first line — except `place`,
  where every line counts.

## 2026.8.31

- **The directory picker opens where you already are.** `d` used to open on an empty input,
  so `/live` meant the filesystem root and reaching a sibling of the directory on screen
  meant typing the whole path from `/`. It now opens holding the current directory, and
  both `live` and `/live` continue from there. Remembered places still lead the list until
  you type. Esc clears the input back to empty, which is how you head somewhere unrelated.

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
