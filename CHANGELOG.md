# Changelog

Versions are calendar dates, `YYYY.M.D` (Eastern time).

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
