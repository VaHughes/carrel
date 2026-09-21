# Reading in carrel

> **carrel** *(n.)* — a small enclosure with a desk, built for one person to sit and read.

Carrel is a reader for markdown. It is not an editor: it never changes a file, it
only shows it to you. This page is an ordinary markdown document, so everything it
describes can be tried right here, while you read. Nothing you do on it can break
anything.

## Getting around

Scroll with the mouse wheel, or with the arrow keys. **Space** moves down a page.
There is a scrollbar along the right edge: drag it, or click anywhere on it to jump
to that part of the document. (If you know vim, `j` and `k` work too, along with the
rest of the motions you would expect.)

| To | Do this |
|---|---|
| Move a line | ↑ or ↓, or the wheel |
| Move a page | Space |
| Jump somewhere | click the scrollbar, or drag it |
| Go back to the last place you were | Backspace or ← |

That table is carrel rendering a markdown table. If the window is too narrow for
it, it turns into one card per row rather than running off the edge.

The row along the bottom of the window is the **hint row**. It shows the few keys
that matter right now and changes as you do things. While you read, it says
`↑/↓ scroll  spc page  / search  o outline  h help`. Each of those is also a
button.

## Everything you can click

This is the part carrel cares most about. You do not need to learn keys to use it;
if something looks like it should respond to a click, it does.

- **Links.** Click a link to another markdown file and it opens here, in the same
  window; Backspace or ← brings you back. Click a link to a web address and the
  address is copied to your clipboard, ready to paste wherever you like. Carrel
  itself never opens a web page (more on that at the end).
- **Headings.** Click any heading to collapse the section beneath it into a single
  line. Click it again to expand. Try it on the heading just above this list, then
  bring it back. (On the keyboard, `za` does the same to the section you are in.)
- **The `▸` markers.** A collapsed section shows a small `▸` beside its heading, as a
  reminder that there is more underneath. Click the marker to expand it again.
- **The buttons along the bottom.** Every item in the hint row is a button. Click
  `h help` and the help sheet opens; click `o outline` and you get a list of every
  heading in the document, which you can click to jump.
- **`⌂` at the top left** takes you back to the list of files. If you opened carrel
  on a single file, it takes you to the files in that file's folder.
- **`≡` at the far right** of the status row opens a menu of everything carrel can
  do, with the key for each item beside it. If you forget a key, this is where to
  look.
- **Right-click anywhere** for a menu of what is under the pointer: a heading offers
  to collapse, a link offers to open or copy, a code block offers to copy itself.
- **`T theme` and `q quit`** in the status row are buttons too. One changes the
  colors; the other leaves.

You can also **drag across text to select it**, and `Ctrl-C` copies the selection.
Double-click selects a word, triple-click a whole paragraph.

## Finding things

Press `/` and start typing. Matches light up as you type, all of them, across the
whole document. Press **Enter** to jump to the first one, `n` for the next and `N`
for the one before. **Esc** clears the search and the highlights with it.

Try it now: press `/`, type `click`, and press Enter. Then make the window narrower
or wider. The highlights stay on their words while the text rewraps around them.
That sounds like a small thing. It is the thing carrel is proudest of, because most
readers lose their place the moment the lines move, and carrel cannot: a match is a
place in the text, not a place on the screen.

`o` opens an **outline** of the document, one row per heading. Type to narrow it,
then click a row or press Enter to jump there.

## Reading comfortably

- `+` and `-` change the **text width**. Prose is kept to a comfortable line length
  and centered, instead of stretching across the whole terminal. Tables and code
  still use the full width.
- `T` cycles the color **theme**. There are several; keep pressing until one suits
  the room you are in. Carrel remembers your choice.
- `h` (or `F1`) opens the **help** sheet, a list of every key. You can type while it
  is open to filter it: type `copy` and only the copying keys remain. `q` or Esc
  closes it.
- `H` hides the hint row when you know your way around, and brings it back when you
  do not. `B` hides the **heading bar** across the top, the one that names the
  section you are currently in.

Some of the text on this page is *italic*, some is **bold**, and some is
`inline code`. Carrel shows each differently, so a document reads the way its
author meant it to.

## Your own files

Run `carrel` with no file name and it lists the markdown around you, newest first,
with the document you were last reading at the top so you can carry on. Click a
file to select it and click again to open it, or move with the arrow keys and press
Enter.

- `i` lets you type part of a name to narrow the list.
- `d` opens a **folder** browser, so you can point carrel at a different folder.
  Carrel remembers the folder you choose.
- `/` searches inside every file in the list, not just their names.

Carrel remembers where you were in each document and reopens it at the same place,
so a long `PLAN.md` picks up where you left it.

## Piping

Anything that prints text can be read in carrel instead of scrolling past in the
terminal:

```sh
git show | carrel
```

A diff arrives as a document with a section per file, so it collapses and searches
like anything else. If a tool or an agent prints markdown to the terminal, pipe it
in the same way and read it here.

## Two things carrel will not do

**It never edits your files.** There is no key that saves, because there is nothing
to save. Read freely.

**It never opens a web address and never fetches anything.** Clicking a link to a
website copies the address to your clipboard instead. That is a deliberate choice: a
markdown file can come from anywhere, and a reader that launches a browser or makes
network requests on your behalf is a reader you have to think twice about before
pointing it at something. Carrel does neither, so you do not.

---

That is the whole of it. Press `q` to leave, and `h` for help any time.
