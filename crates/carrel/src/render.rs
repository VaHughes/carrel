//! The paint loop. Rows in, cells out.
//!
//! **Never uses `Paragraph` or `Wrap`.** Three open ratatui panics are
//! reachable from markdown through that path — #2679 (double-width graphemes),
//! #2695 (a `Buffer::diff` regression introduced *in* 0.30.2), #925 (ZWJ) — its
//! scroll offset is `u16` so it cannot address past row 65,535, and it re-wraps
//! from the top every frame. We own wrapping already, so rows go straight into
//! the `Buffer`.
//!
//! # Paint order, which is not arbitrary
//!
//! 1. quote bars — every row
//! 2. prefix — first row of the block only
//! 3. text — split by inline style runs
//! 4. highlights — LAST, as style repainted over cell rects
//!
//! Step 4 must come last and must be `set_style`, never span splitting:
//! `architecture.md` §8 item 16.

use std::borrow::Cow;
use std::fmt::Write as _;

use carrel_core::{BlockIdx, NodeKind, Row, RowKind, cols_for_doc_range, display_width};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use std::collections::HashMap;

use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;

use crate::app::{App, Mode, Screen};

/// A visible link span, for the OSC 8 post-draw pass in `main.rs`.
///
/// ratatui 0.30 has no hyperlink support, so after each frame the visible
/// link text is re-emitted wrapped in `ESC ] 8`. Terminals without OSC 8
/// ignore it — that is the graceful degradation.
#[derive(Debug)]
pub struct OscLink {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub url: String,
}
use crate::home::{Home, HomeMode};
use crate::theme;

pub fn draw(frame: &mut Frame, app: &App) {
    draw_with_links(frame, app, &mut Vec::new());
}

/// Draw, collecting visible link spans for the OSC 8 pass.
pub fn draw_with_links(frame: &mut Frame, app: &App, links: &mut Vec<OscLink>) {
    draw_full(frame, app, links, &mut HashMap::new());
}

/// Draw with everything: link collection and ready-image protocols.
///
/// The protocol map is the ONLY place `ratatui-image` state reaches painting;
/// `app` carries plain dimensions and nothing else.
#[allow(clippy::implicit_hasher)] // an internal map, never a generic API
pub fn draw_full(
    frame: &mut Frame,
    app: &App,
    links: &mut Vec<OscLink>,
    images: &mut HashMap<carrel_core::BlockIdx, StatefulProtocol>,
) {
    links.clear();
    if let Screen::Home(h) = &app.screen {
        let area = frame.area();
        if area.width < 2 || area.height < 3 {
            frame.buffer_mut().set_stringn(
                0,
                0,
                "carrel: window too small",
                area.width as usize,
                theme::status(),
            );
            return;
        }
        frame.buffer_mut().set_style(area, theme::body());
        draw_home(frame, app, h);
        if app.help.is_some() {
            paint_help(frame, app);
        }
        if app.outline.is_some() {
            paint_outline(frame, app);
        }
        return;
    }

    let area = frame.area();
    if area.width < 2 || area.height < 2 {
        frame.buffer_mut().set_stringn(
            0,
            0,
            "carrel: window too small",
            area.width as usize,
            theme::status(),
        );
        return;
    }

    // A themed page paints its own background; `terminal` inherits.
    frame.buffer_mut().set_style(area, theme::body());

    // Derived from the SAME function the layout width came from, or paint and
    // wrapping would disagree about how much room the text has.
    let (_tw, bw, th) = App::text_size(
        area.width,
        area.height,
        app.hints,
        app.band(),
        app.max_width,
    );
    // The FULL text area, not the measure column: `paint_rows` gives each
    // block its own column inside it, so a table may be wider than prose
    // without being clipped. Prose lands at `text_x`, the same function
    // `doc_span_at` hit-tests against — and the top edge comes through
    // `text_y` for exactly the same reason, so a click cannot resolve to a
    // different byte than the one under the pointer.
    let text = Rect::new(crate::app::PAD_LEFT, app.text_y(), bw, th);
    // The bar keeps the true right edge but aligns its track with the text
    // rows, so thumb geometry and the drag hit-test share one coordinate.
    let bar = Rect::new(area.width.saturating_sub(1), app.text_y(), 1, th);
    let status_y = area.height.saturating_sub(if app.hints { 2 } else { 1 });
    let status = Rect::new(0, status_y, area.width, 1);

    paint_rows(frame, app, text, links, images);
    paint_scrollbar(frame, app, bar);
    paint_status(frame, app, status);
    paint_breadcrumb(frame, app, text);
    if app.hints {
        paint_footer(
            frame,
            app,
            Rect::new(0, area.height.saturating_sub(1), area.width, 1),
        );
    }
    if app.help.is_some() {
        paint_help(frame, app);
    }
    if app.outline.is_some() {
        paint_outline(frame, app);
    }
}

/// The outline picker: headings indented by level, the selection styled,
/// the list window scrolled to keep the selection visible. Content derives
/// from the document every frame — nothing here can be stale.
fn paint_outline(frame: &mut Frame, app: &App) {
    let Some(picker) = &app.outline else { return };
    let matches = app.outline_matches();
    let area = frame.area();
    if area.width < 20 || area.height < 4 {
        return;
    }
    let w = 52u16.min(area.width.saturating_sub(2));
    let h = (u16::try_from(matches.len()).unwrap_or(u16::MAX) + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let x = (area.width - w) / 2;
    let y = (area.height - h) / 2;
    let buf = frame.buffer_mut();

    let blank = " ".repeat(w as usize);
    for py in y..y + h {
        buf.set_stringn(x, py, &blank, w as usize, theme::status());
    }
    let bar = "─".repeat(w as usize);
    let title = if picker.filter.is_empty() {
        format!("┌ outline {bar}")
    } else {
        format!("┌ outline /{} {bar}", picker.filter)
    };
    buf.set_stringn(x, y, title, w as usize, theme::status());
    buf.set_stringn(
        x,
        y + h - 1,
        format!("└ type to filter · ↵ jump · esc close {bar}"),
        w as usize,
        theme::status(),
    );

    let inner_h = usize::from(h - 2);
    // Keep the selection inside the window.
    let first = picker.selected.saturating_sub(inner_h.saturating_sub(1));
    let first = first.min(matches.len().saturating_sub(inner_h));
    for (i, block) in matches.iter().skip(first).take(inner_h).enumerate() {
        let py = y + 1 + u16::try_from(i).unwrap_or(u16::MAX);
        let node = app.doc.node_for_block(*block);
        let level = match node.kind {
            NodeKind::Heading { level } => level,
            _ => 1,
        };
        let label = &app.doc.text[node.doc.start as usize..node.doc.end as usize];
        let indent = "  ".repeat(usize::from(level.saturating_sub(1)));
        let selected = first + i == picker.selected;
        let marker = if selected { "▸" } else { " " };
        let style = if selected {
            theme::selected()
        } else {
            theme::status()
        };
        let line = format!(" {marker} {indent}{label}");
        buf.set_stringn(x, py, line, w as usize, style);
    }
}

/// The key-binding sheet: a centred panel over the page. Content comes from
/// `keys::{READER_HELP, HOME_HELP}` — the drift test in keys.rs keeps those
/// tables honest, so this painter never lies either. The state's scroll
/// saturates; the CLAMP against the sheet's length happens here, where the
/// content lives.
fn paint_help(frame: &mut Frame, app: &App) {
    let table = if app.is_home() {
        crate::keys::HOME_HELP
    } else {
        crate::keys::READER_HELP
    };
    let area = frame.area();
    if area.width < 20 || area.height < 4 {
        return; // nothing legible fits; the toggle still works
    }
    // 52 = 4 indent + 18 key column + 1 gap + 29 description columns — wide
    // enough that no row in either table truncates (the test pins the
    // longest one).
    let w = 52u16.min(area.width.saturating_sub(2));
    let h = (u16::try_from(table.len()).unwrap_or(u16::MAX) + 2).min(area.height.saturating_sub(2));
    let x = (area.width - w) / 2;
    let y = (area.height - h) / 2;
    let buf = frame.buffer_mut();

    let blank = " ".repeat(w as usize);
    for py in y..y + h {
        buf.set_stringn(x, py, &blank, w as usize, theme::status());
    }
    let bar = "─".repeat(w as usize);
    buf.set_stringn(
        x,
        y,
        format!("┌ carrel — keys {bar}"),
        w as usize,
        theme::status(),
    );
    buf.set_stringn(
        x,
        y + h - 1,
        format!("└ h · q · Esc close    j k scroll {bar}"),
        w as usize,
        theme::status(),
    );

    let inner_h = usize::from(h - 2);
    let max_scroll = table.len().saturating_sub(inner_h);
    let scroll = usize::from(app.help.unwrap_or(0)).min(max_scroll);
    for (i, (key, desc)) in table.iter().skip(scroll).take(inner_h).enumerate() {
        let py = y + 1 + u16::try_from(i).unwrap_or(u16::MAX);
        if *key == "§" {
            buf.set_stringn(x, py, format!("  {desc}"), w as usize, theme::selected());
        } else {
            let line = format!("    {key:<18} {desc}");
            buf.set_stringn(x, py, line, w as usize, theme::status());
        }
    }
}

/// Paint a math block's art, returning the next free row, or `None` when this
/// block is not rendered math and should fall through to the ordinary paths.
fn paint_math(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    block: BlockIdx,
    skip: u32,
    mut y: u16,
) -> Option<u16> {
    let node = app.doc.node_for_block(block);
    let avail = area.width.saturating_sub(node.indent);
    let form = app.math_form(block, avail);
    if form == crate::app::MathForm::Source {
        return None;
    }
    let art = app.math_art.get(&block)?;
    let chosen = if form == crate::app::MathForm::Display {
        &art.display
    } else {
        &art.inline
    };
    let x = area.x + node.indent.min(area.width.saturating_sub(1));
    let w = usize::from(avail);
    for line in chosen
        .rows
        .iter()
        .skip(usize::try_from(skip).unwrap_or(usize::MAX))
    {
        if y >= area.bottom() {
            break;
        }
        frame
            .buffer_mut()
            .set_stringn(x, y, line, w, crate::theme::marker());
        y += 1;
    }
    let len = u32::try_from(chosen.rows.len()).unwrap_or(u32::MAX);
    let trailing = app.layout.height(block).saturating_sub(len.max(skip));
    Some(
        y.saturating_add(u16::try_from(trailing).unwrap_or(u16::MAX))
            .min(area.bottom()),
    )
}

/// Paint a frontmatter block as a card, returning the next free row.
///
/// The `╭ │ ╰` rule is DECORATION and is not in the text — the same standing
/// the table `│` separators have. It sits in the two-cell gutter the node's
/// `indent` already reserved at parse, so values wrap inside the card rather
/// than under the rule.
fn paint_metadata_card(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    block: BlockIdx,
    key_col: u16,
    skip: u32,
    mut y: u16,
) -> u16 {
    let node = app.doc.node_for_block(block);
    let body = &app.doc.text[node.doc.start as usize..node.doc.end as usize];
    let text_lines: Vec<&str> = body.lines().collect();
    let last = text_lines.len().saturating_sub(1);
    let avail = usize::from(area.width.saturating_sub(node.indent));
    let text_x = area.x + node.indent;

    for (i, line) in text_lines
        .iter()
        .enumerate()
        .skip(usize::try_from(skip).unwrap_or(usize::MAX))
    {
        if y >= area.bottom() {
            break;
        }
        let rule = if i == 0 {
            '╭'
        } else if i == last {
            '╰'
        } else {
            '│'
        };
        let buf = frame.buffer_mut();
        buf.set_stringn(area.x, y, rule.to_string(), 1, crate::theme::dim());
        match line.split_once(':').or_else(|| line.split_once('=')) {
            // Only a flush line is a key line; an indented one is a nested
            // value, a list item, or a block scalar, and prints raw.
            Some((key, val)) if !line.starts_with([' ', '\t', '#', '-']) => {
                let key = key.trim_end();
                let key_w = display_width(key);
                buf.set_stringn(text_x, y, key, avail, crate::theme::meta_key());
                let val_x = text_x
                    .saturating_add(key_w)
                    .saturating_add(key_col.saturating_sub(key_w) + 1);
                if val_x < area.right() {
                    buf.set_stringn(
                        val_x,
                        y,
                        val.trim_start(),
                        usize::from(area.right() - val_x),
                        crate::theme::meta_value(),
                    );
                }
            }
            _ => {
                buf.set_stringn(text_x, y, *line, avail, crate::theme::meta_value());
            }
        }
        y += 1;
    }

    // Consume the block's remaining layout rows, spacing included.
    let painted = u32::try_from(text_lines.len()).unwrap_or(u32::MAX);
    let trailing = app.layout.height(block).saturating_sub(painted.max(skip));
    y.saturating_add(u16::try_from(trailing).unwrap_or(u16::MAX))
        .min(area.bottom())
}

/// Where a block's own column starts, and how wide it is, inside the full
/// text area.
///
/// Prose sits in the fixed measure column so consecutive paragraphs line up.
/// A block with an intrinsic width of its own may be wider than that; a table
/// centres on the page axis by its aligned width, and anything else keeps the
/// prose column's left edge and simply extends right. Code is deliberately in
/// the second group: centring it by its longest line would make consecutive
/// blocks jitter horizontally, which reads worse than the asymmetry.
fn block_area(app: &App, node: &carrel_core::Node, full: Rect) -> Rect {
    let prose_x = app.text_x_now();
    let budget = app.layout.block_width(&node.kind);
    if budget <= app.text_w() {
        // Bound to the measure: the fixed column.
        return Rect::new(prose_x, full.y, budget.min(full.width), full.height);
    }
    // A bleed kind. Centre a table by the width it actually occupies.
    let x = match &node.kind {
        NodeKind::Table { cols, .. } if !cols.is_empty() => {
            let aligned = cols.iter().map(|&c| u32::from(c)).sum::<u32>()
                + 3 * (cols.len() as u32 - 1)
                + u32::from(node.indent);
            let aligned = u16::try_from(aligned).unwrap_or(u16::MAX).min(full.width);
            if aligned > app.text_w() {
                full.x + (full.width - aligned) / 2
            } else {
                prose_x
            }
        }
        _ => prose_x,
    };
    // Never run off the right edge, and never start left of the text area.
    let x = x.max(full.x).min(full.right().saturating_sub(1));
    Rect::new(x, full.y, full.right() - x, full.height)
}

fn paint_rows(
    frame: &mut Frame,
    app: &App,
    full: Rect,
    links: &mut Vec<OscLink>,
    images: &mut HashMap<BlockIdx, StatefulProtocol>,
) {
    // One buffer for the whole frame: after the first block this allocates
    // nothing. There is no row cache — see layout.rs.
    let mut rows: Vec<Row> = Vec::new();
    let mut y = full.y;
    let mut block = app.layout.block_at_row(app.view.scroll_row);
    let mut skip = app
        .view
        .scroll_row
        .saturating_sub(app.layout.row_start(block));

    while y < full.bottom() && block.get() < app.doc.block_count() {
        // A folded-away block owns zero rows; it is not on the page. The
        // layout already excluded it from every row computation — paint
        // must walk past it or it would draw rows the layout never counted.
        if app.layout.height(block) == 0 {
            block = BlockIdx(block.0 + 1);
            continue;
        }
        // Each block paints into ITS OWN column — prose in the measure, a
        // wide table across the page. Shadowing `area` here means every
        // painter below sees the block's geometry and none of them has to
        // know the measure exists.
        let area = block_area(app, app.doc.node_for_block(block), full);
        // A rendered mermaid diagram paints its art lines instead of the
        // block's wrapped source — properly line-skipped on partial scroll,
        // which text can do and pixels cannot. Wider-than-viewport art
        // right-clips like a wide code line.
        if app.show_rendered
            && let Some(art) = app.diagram_art.get(&block)
        {
            let node = app.doc.node_for_block(block);
            let x = area.x + node.indent.min(area.width.saturating_sub(1));
            let w = usize::from(area.width.saturating_sub(node.indent));
            let total = app.layout.height(block);
            for line in art.iter().skip(usize::try_from(skip).unwrap_or(usize::MAX)) {
                if y >= area.bottom() {
                    break;
                }
                frame
                    .buffer_mut()
                    .set_stringn(x, y, line, w, crate::theme::marker());
                y += 1;
            }
            // Consume the block's remaining layout rows (spacing included):
            // the art loop already consumed `art_len - skip` visible lines.
            let art_len = u32::try_from(art.len()).unwrap_or(u32::MAX);
            let trailing = total.saturating_sub(art_len.max(skip));
            y = y
                .saturating_add(u16::try_from(trailing).unwrap_or(u16::MAX))
                .min(area.bottom());
            skip = 0;
            block = BlockIdx(block.0 + 1);
            continue;
        }

        // Rendered math paints its art lines, line-skipped on partial scroll
        // exactly like a diagram. Which form is chosen is `App::math_form`'s
        // call, and paint MUST ask it rather than deciding independently, or
        // height and paint disagree about how many rows this block occupies.
        if let Some(next_y) = paint_math(frame, app, area, block, skip, y) {
            y = next_y;
            skip = 0;
            block = BlockIdx(block.0 + 1);
            continue;
        }

        // Frontmatter paints as a card: a left rule in the gutter the node's
        // `indent` already reserved, the key padded to `key_col`, then the
        // value. The `╭ │ ╰` glyphs are DECORATION and are not in the text —
        // the same standing the table `│` separators have.
        if let NodeKind::Metadata { key_col } = app.doc.node_for_block(block).kind {
            y = paint_metadata_card(frame, app, area, block, key_col, skip, y);
            skip = 0;
            block = BlockIdx(block.0 + 1);
            continue;
        }

        // A ready image paints once, as a widget into the visible slice of
        // its block — not row by row. Partially scrolled blocks render a
        // top-anchored crop into the intersection: the documented v1
        // limitation, and still better than glyphs.
        if let Some(proto) = images.get_mut(&block) {
            let node = app.doc.node_for_block(block);
            let total = app.layout.height(block);
            let content = app.layout.content_height(&app.doc, block);
            let remaining = u32::from(area.bottom().saturating_sub(y));
            // The widget gets only the CONTENT rows — the trailing spacing row
            // stays blank, or the picture stretches into the gap.
            let content_visible = content.saturating_sub(skip).min(remaining);
            if content_visible > 0 {
                let x = area.x + node.indent.min(area.width.saturating_sub(1));
                let w = area.width.saturating_sub(node.indent);
                let rect = Rect::new(x, y, w, u16::try_from(content_visible).unwrap_or(u16::MAX));
                frame.render_stateful_widget(StatefulImage::new(), rect, proto);
            }
            let consumed = total.saturating_sub(skip).min(remaining);
            y += u16::try_from(consumed).unwrap_or(u16::MAX);
            skip = 0;
            block = BlockIdx(block.0 + 1);
            continue;
        }

        app.layout.rows_for(&app.doc, block, &mut rows);
        let first_y = y;
        for row in rows.iter().skip(skip as usize) {
            if y >= area.bottom() {
                break;
            }
            paint_row(frame, app, block, row, area, y, links);
            y += 1;
        }
        // A folded heading wears its state: a gutter marker and a trailing
        // ellipsis. Decoration only — neither is in the text, so search and
        // selection never see them.
        let node = app.doc.node_for_block(block);
        if skip == 0
            && y > first_y
            && matches!(node.kind, NodeKind::Heading { .. })
            && app.folded.contains(&node.id)
        {
            let buf = frame.buffer_mut();
            buf.set_stringn(area.x.saturating_sub(2), first_y, "▸", 1, theme::dim());
            let text = &app.doc.text[node.doc.start as usize..node.doc.end as usize];
            let after = area
                .x
                .saturating_add(node.indent)
                .saturating_add(carrel_core::display_width(text))
                .saturating_add(1);
            if after < area.right() {
                buf.set_stringn(after, first_y, "…", 1, theme::dim());
            }
        }
        skip = 0;
        block = BlockIdx(block.0 + 1);
    }
}

#[allow(clippy::too_many_lines)]
fn paint_row(
    frame: &mut Frame,
    app: &App,
    block: BlockIdx,
    row: &Row,
    area: Rect,
    y: u16,
    links: &mut Vec<OscLink>,
) {
    let node = app.doc.node_for_block(block);

    // Card mode: an overflowing table lays out as label/value cards instead
    // of wrapping in place, unless `t` has switched to the padded-wrap
    // rendering. Recomputed at paint, never stored — the same rule `Layout`
    // used to decide the row stream in the first place.
    let cards = matches!(node.kind, NodeKind::Table { .. })
        && !app.layout.wrap_tables()
        && crate::layout::table_overflows(node, app.layout.width());

    let base = match &node.kind {
        NodeKind::Heading { level } => theme::heading(*level),
        // An alert's label line: the kind's colour, bold.
        NodeKind::AlertLabel { kind } => theme::alert(*kind).add_modifier(Modifier::BOLD),
        // An image rendering as alt text: loading, failed, or remote.
        NodeKind::Image { .. } => theme::dim(),
        // A table's header is its first logical line.
        NodeKind::Table { .. } => {
            let text = app.doc.block_text(block);
            let first_nl = text
                .find('\n')
                .map_or(node.doc.end, |i| node.doc.start + i as u32);
            if row.doc.end <= first_nl {
                Style::default().add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default()
            }
        }
        _ => Style::default(),
    };

    {
        let buf = frame.buffer_mut();

        // 1. Quote bars — EVERY row. A prefix is first-row-only; a quote bar is
        //    not, or a wrapped blockquote loses its bar partway down. Inside
        //    a GFM alert the bars take the kind's colour.
        let bar = match node.alert {
            Some(kind) => theme::alert(kind),
            None => theme::quote_bar(),
        };
        for &col in &node.quote_cols {
            let x = area.x + col;
            if x < area.right() {
                buf.set_stringn(x, y, "│ ", 2, bar);
            }
        }

        // 1a. Decoration rows carry no text: a card rule, an image
        //     placeholder, or a block's trailing gap. Only a card rule
        //     (anchored away from the block's own start — Task 3's
        //     discriminator) paints anything; image and gap rows fall
        //     straight through to the return with just their quote bar.
        if let RowKind::Decoration = row.kind {
            if cards && row.doc.start != node.doc.start {
                let x = area.x + node.indent;
                let w = area.right().saturating_sub(x).min(app.layout.width());
                buf.set_stringn(x, y, "─".repeat(w as usize), w as usize, theme::dim());
            }
            return;
        }

        // 2. Prefix — first row only, painted INTO the reserved inset so
        //    continuation rows hang under the text rather than under the bullet.
        if let (
            RowKind::Text {
                first_in_block: true,
                ..
            },
            Some(p),
        ) = (row.kind, node.prefix.as_ref())
        {
            let x = area.x + row.indent.saturating_sub(p.width);
            if x < area.right() {
                buf.set_stringn(x, y, &*p.text, p.width as usize, theme::marker());
            }
        }

        // 2b. The continuation marker. Punctuation, not content — and only for
        //     code, where losing the indentation context actively misleads.
        //     The core reserved CONTINUATION_COLS in row.indent for exactly this.
        if let RowKind::Text {
            continued: true, ..
        } = row.kind
            && matches!(node.kind, NodeKind::CodeBlock { .. })
        {
            let x = area.x + row.indent.saturating_sub(carrel_core::CONTINUATION_COLS);
            if x < area.right() {
                buf.set_stringn(x, y, "↳ ", 2, theme::dim());
            }
        }

        // 2c. Card gutter label — body value rows only. The column is
        //     recovered from `cell_starts`: the greatest cell start at or
        //     before this row's, mod the column count, names the header cell
        //     whose text becomes the (dim, truncated) label.
        if cards
            && let NodeKind::Table { cols, cell_starts } = &node.kind
            && let RowKind::Text {
                continued: false, ..
            } = row.kind
        {
            let ncols = cols.len();
            let header_end = cell_starts.get(ncols).copied().unwrap_or(node.doc.end);
            if row.doc.start >= header_end {
                // Rank of the greatest cell_start <= row.doc.start. Defensive
                // `saturating_sub`: `partition_point` returning 0 would mean
                // `cell_starts[0] != node.doc.start`, which never happens
                // today, but this must not underflow if that ever changes.
                let idx = cell_starts
                    .partition_point(|&cs| cs <= row.doc.start)
                    .saturating_sub(1);
                let c = idx % ncols;
                let ls = cell_starts[c] as usize;
                let le = if c + 1 < ncols {
                    cell_starts[c + 1] as usize
                } else {
                    header_end as usize
                };
                let label = app.doc.text[ls..le.max(ls)].trim_end();
                let gutter = row.indent.saturating_sub(node.indent) as usize;
                let avail = gutter.saturating_sub(2);
                let (nx, _) = buf.set_stringn(area.x + node.indent, y, label, avail, theme::dim());
                if usize::from(carrel_core::display_width(label)) > avail {
                    buf.set_stringn(nx, y, "…", 1, theme::dim());
                }
            }
        }

        // 3. Text. Code blocks split by semantic syntax tokens; everything
        //    else splits by inline style runs. The two never coexist on one
        //    node — code blocks have empty `inlines`, prose has empty `tokens`.
        let code_block = matches!(node.kind, NodeKind::CodeBlock { .. });
        let code_base = theme::token(carrel_core::TokenKind::Plain);
        // Lazy: the first paint of a code block computes its tokens.
        let tokens = if code_block {
            app.doc.tokens(block)
        } else {
            &[]
        };
        let mut x = area.x + row.indent;
        let mut at = row.doc.start;
        while at < row.doc.end && x < area.right() {
            let (style, end) = if code_block {
                run_at(tokens, at, row.doc.end, code_base, |t| theme::token(t.kind))
            } else {
                run_at(&node.inlines, at, row.doc.end, base, |i| {
                    base.patch(theme::inline(i.style))
                })
            };
            let s = &app.doc.text[at as usize..end as usize];
            // Table cells are separated by a synthetic '\t' that the unwritten
            // table layout will consume. `set_stringn` SKIPS control characters,
            // so leaving it would butt two cells together ("resizeNo shipping").
            // A space is the honest placeholder until tables are designed (Q15).
            let s: &str = &if s.contains('\t') {
                Cow::Owned(s.replace('\t', " "))
            } else {
                Cow::Borrowed(s)
            };
            let (nx, _) = buf.set_stringn(x, y, s, (area.right() - x) as usize, style);
            x = nx;
            at = end;
        }
    }

    // 3a. Table column separators: a dim │ in the middle of each 2-cell gap,
    //     painted over synthetic padding. Only on unwrapped rows — a wrapped
    //     table row's columns no longer line up, and its continuation carries
    //     none of the cumulative offsets. In card mode the columns are no
    //     longer aligned on body rows at all (one column per row, under the
    //     gutter), so this pass runs only on the legend — with a `·` instead
    //     of a `│`, since there is no cell content on either side of it.
    if let NodeKind::Table { cols, cell_starts } = &node.kind
        && let RowKind::Text {
            continued: false, ..
        } = row.kind
    {
        let ncols = cols.len();
        let header_end = cell_starts.get(ncols).copied().unwrap_or(node.doc.end);
        // Same shape as stage 2c's body-row test, so the two boundary checks
        // cannot drift apart: a row is body once it starts at or past the
        // first body cell.
        let is_body = row.doc.start >= header_end;
        if !cards || !is_body {
            let sep = if cards { "·" } else { "│" };
            let row_text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
            let row_w = carrel_core::display_width(row_text);
            let buf = frame.buffer_mut();
            let mut cum = 0u16;
            for w in cols.iter().take(cols.len().saturating_sub(1)) {
                cum = cum.saturating_add(*w);
                let off = cum.saturating_add(1); // the middle of the 3-cell gap
                cum = cum.saturating_add(3);
                if off >= row_w {
                    break; // this visual row wrapped before the gap
                }
                let x = area.x + row.indent + off;
                if x < area.right() {
                    buf.set_stringn(x, y, sep, 1, theme::quote_bar());
                }
            }
        }
    }

    // 3b. Links on this row: collect spans for the OSC 8 pass, and repaint
    //     the one `Tab` selected. The selected link keeps its highlight
    //     instead of an OSC wrapper — you don't need to click what Enter
    //     already follows.
    {
        let row_text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
        for inline in node.inlines.iter().filter(|i| i.link.is_some()) {
            if inline.doc.end <= row.doc.start || inline.doc.start >= row.doc.end {
                continue;
            }
            let clamped = inline.doc.start.max(row.doc.start)..inline.doc.end.min(row.doc.end);
            let (c0, c1) = cols_for_doc_range(row_text, row.doc.start, row.indent, &clamped);
            if c1 <= c0 {
                continue;
            }
            let x0 = area.x + c0;
            let w = (c1 - c0).min(area.right().saturating_sub(x0));
            if w == 0 {
                continue;
            }
            if inline.link == app.selected_link {
                frame
                    .buffer_mut()
                    .set_style(Rect::new(x0, y, w, 1), theme::link_selected());
            } else if let Some(id) = inline.link {
                // A wikilink's stored target is a note name, not a URI: OSC 8
                // gets a file:// URI when the open-time resolution found the
                // note, and nothing at all when it didn't — a terminal cannot
                // click a name.
                let raw = if app.doc.is_wikilink(id) {
                    match app.wiki.get(&id) {
                        Some(p) => format!("file://{}", p.display()),
                        None => continue,
                    }
                } else {
                    app.doc.links[id.0 as usize].to_string()
                };
                // Strip control characters so a hostile URL cannot smuggle
                // escape sequences into the terminal through the OSC wrapper,
                // and skip absurd URLs outright — the OSC pass re-emits every
                // frame, and terminals cap near 2 KiB anyway.
                let url: String = raw.chars().filter(|c| !c.is_control()).collect();
                if url.len() > 2048 {
                    continue;
                }
                // The TEXT is sanitised too, not just the URL: the OSC pass
                // prints it raw to stdout, outside ratatui's control-character
                // filtering, so a hostile document could otherwise smuggle
                // escapes through the link text itself.
                let text: String = app.doc.text[clamped.start as usize..clamped.end as usize]
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                links.push(OscLink {
                    x: x0,
                    y,
                    text,
                    url,
                });
            }
        }
    }

    // 3.5. The mouse selection — same style-repaint mechanism as search
    // highlights, painted before them so the current match stays visible
    // inside a selection.
    if let Some(sel) = &app.selection {
        let clamped = sel.start.max(row.doc.start)..sel.end.min(row.doc.end);
        if clamped.start < clamped.end {
            let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
            let (c0, c1) = cols_for_doc_range(text, row.doc.start, row.indent, &clamped);
            if c1 > c0 {
                let x0 = area.x + c0;
                let w = (c1 - c0).min(area.right().saturating_sub(x0));
                if w > 0 {
                    frame
                        .buffer_mut()
                        .set_style(Rect::new(x0, y, w, 1), theme::selection());
                }
            }
        }
    }

    // 4. Highlights LAST — repaint style over cell rects. Never split spans.
    let Some(m) = app.matches.as_ref() else {
        return;
    };
    let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
    let hits: Vec<(usize, u16, u16)> = m
        .intersecting(&row.doc)
        .filter_map(|(i, r)| {
            let clamped = r.start.max(row.doc.start)..r.end.min(row.doc.end);
            let (c0, c1) = cols_for_doc_range(text, row.doc.start, row.indent, &clamped);
            (c1 > c0).then_some((i, c0, c1))
        })
        .collect();

    let buf = frame.buffer_mut();
    for (i, c0, c1) in hits {
        let x0 = area.x + c0;
        let w = (c1 - c0).min(area.right().saturating_sub(x0));
        if w == 0 {
            continue;
        }
        let style = if Some(i) == m.current {
            theme::match_current()
        } else {
            theme::match_normal()
        };
        buf.set_style(Rect::new(x0, y, w, 1), style);
    }
}

/// The styled run covering `at`, and where it ends within the row.
///
/// Works over anything with a doc range — inline style runs for prose,
/// semantic tokens for code. Stops at the NEXT run's start even when `at` is
/// unstyled, or a styled run later in the row would be painted plain.
fn run_at<T>(
    runs: &[T],
    at: u32,
    row_end: u32,
    plain: Style,
    style_of: impl Fn(&T) -> Style,
) -> (Style, u32)
where
    T: HasDocRange,
{
    let run = runs
        .iter()
        .find(|r| r.range().start <= at && at < r.range().end);
    let end = match run {
        Some(r) => r.range().end.min(row_end),
        None => runs
            .iter()
            .find(|r| r.range().start > at)
            .map_or(row_end, |r| r.range().start.min(row_end)),
    };
    (run.map_or(plain, style_of), end)
}

/// The one thing `run_at` needs from a run: where it sits in doc space.
trait HasDocRange {
    fn range(&self) -> &std::ops::Range<u32>;
}
impl HasDocRange for carrel_core::Inline {
    fn range(&self) -> &std::ops::Range<u32> {
        &self.doc
    }
}
impl HasDocRange for carrel_core::Token {
    fn range(&self) -> &std::ops::Range<u32> {
        &self.doc
    }
}

fn paint_scrollbar(frame: &mut Frame, app: &App, area: Rect) {
    // Painted from the SAME geometry the mouse hit-testing uses
    // (`keys::thumb_geometry`), so users grab exactly what they see, and the
    // thumb touches the bottom of the bar at the bottom of the document.
    let total = app.layout.total_rows();
    let (top, len) = crate::keys::thumb_geometry(area.height, total, app.view.scroll_row);

    // Match ticks — the overview ruler (Q8). Derived per frame from match
    // BYTE offsets: block row start plus a byte-fraction estimate within the
    // block, O(1) per match. Cell-of-a-40-row-track precision is what the
    // eye can use; nothing display-shaped is ever stored.
    let mut tick = vec![false; area.height as usize];
    if let Some(m) = &app.matches
        && area.height > 0
        && total > 1
    {
        for r in &m.ranges {
            let b = app.doc.block_at_doc(carrel_core::DocByte(r.start));
            let node = app.doc.node_for_block(b);
            let bytes = u64::from(node.doc.end.saturating_sub(node.doc.start)).max(1);
            let h = u64::from(app.layout.content_height(&app.doc, b).saturating_sub(1));
            let within = u64::from(r.start.saturating_sub(node.doc.start)) * h / bytes;
            let row = app.layout.row_start(b) + u32::try_from(within).unwrap_or(0);
            let cell = u64::from(row) * u64::from(area.height - 1) / u64::from(total - 1);
            if let Some(t) = tick.get_mut(usize::try_from(cell).unwrap_or(0)) {
                *t = true;
            }
        }
    }

    let buf = frame.buffer_mut();
    for dy in 0..area.height {
        let (sym, style) = if dy >= top && dy < top.saturating_add(len) {
            ("█", theme::marker())
        } else if tick[dy as usize] {
            ("·", theme::lamp())
        } else {
            ("│", theme::dim())
        };
        buf.set_stringn(area.x, area.y + dy, sym, 1, style);
    }
}

fn paint_status(frame: &mut Frame, app: &App, area: Rect) {
    let left = match &app.mode {
        Mode::Search { input, .. } => format!("/{input}"),
        // A one-shot note (a failed follow, an external URL) outranks the
        // filename until the next action clears it.
        Mode::Normal => app.note.clone().unwrap_or_else(|| app.path.clone()),
    };
    let right = if let Some(id) = app.selected_link {
        // The selected link's destination, always visible for copying.
        app.doc.links[id.0 as usize].to_string()
    } else if let Some((i, n)) = app
        .matches
        .as_ref()
        .and_then(carrel_core::Matches::position)
    {
        format!("{i} of {n}")
    } else {
        // Percent of the SCROLLABLE range: the bottom of the document must
        // read 100%, or the reader concludes scrolling is broken. scroll/total
        // could never reach 100 — the viewport's own height was the shortfall.
        let max = app.layout.max_scroll(app.text_h());
        let pct = if max == 0 {
            100
        } else {
            (u64::from(app.view.scroll_row) * 100 / u64::from(max)).min(100)
        };
        // The exit key, visibly: q returns to the home screen when one is
        // behind this document, and quits when the file was opened directly.
        // T is here for the same reason q is — a key nobody can see is a
        // feature nobody has (field note, twice now).
        let exit = if app.home_stash.is_some() {
            "q home"
        } else {
            "q quit"
        };
        // "how much is left" is the question a reader actually has; the
        // percentage answers "where am I". Both, when there is time worth
        // mentioning — `minutes_left` stays quiet under a minute.
        match app.minutes_left() {
            Some(m) => format!("{pct}% · {m} min left · T theme · {exit}"),
            None => format!("{pct}% · T theme · {exit}"),
        }
    };

    let buf = frame.buffer_mut();
    buf.set_style(area, theme::status());
    let lx = fold_lamp(buf, app.hints, area);
    buf.set_stringn(lx, area.y, &left, area.width as usize, theme::status());
    let rw = u16::try_from(right.chars().count()).unwrap_or(0);
    let rx = area.right().saturating_sub(rw);
    if rx > lx + u16::try_from(left.chars().count()).unwrap_or(0) {
        buf.set_stringn(rx, area.y, &right, rw as usize, theme::status());
    }
}

/// When the hints are hidden, the status row's left edge shows the folded,
/// darkened lamp — `╰○` — which is both the state and the "turn me back on"
/// affordance (clicking it, or `H`, re-lights it). Returns the x where the
/// status text starts.
fn fold_lamp(buf: &mut ratatui::buffer::Buffer, hints: bool, area: Rect) -> u16 {
    if hints {
        return area.x;
    }
    buf.set_stringn(area.x, area.y, "╰○ ", 3, theme::dim());
    area.x + 3
}

/// Advance-and-paint for the footer's segments: writes `s` at `*x`, clipped
/// to the row, and moves `*x` past it.
fn put(buf: &mut ratatui::buffer::Buffer, x: &mut u16, y: u16, right: u16, s: &str, style: Style) {
    if *x >= right {
        return;
    }
    buf.set_stringn(*x, y, s, (right - *x) as usize, style);
    *x += carrel_core::display_width(s);
}

/// The lamplight row: `╭● word  ░ key label · key label ░` — the footer spec
/// §1/§4. Trims WHOLE hints right-to-left (a pinned trailing `h` hint
/// survives longest), then the `░` caps, then the mode word — never a cut
/// inside a hint. Colours are theme slots only: lamp for the bulb and keys,
/// wordmark for the mode word, dim for labels and furniture.
fn paint_footer(frame: &mut Frame, app: &App, area: Rect) {
    use carrel_core::display_width;
    let f = crate::footer::of(app);
    let w = area.width as usize;
    let pinned = f.hints.last().is_some_and(|(k, _)| *k == "h");
    let shown = |keep: usize| -> Vec<(&str, &str)> {
        if pinned && keep >= 1 {
            let mut v: Vec<_> = f.hints[..keep - 1].to_vec();
            v.push(*f.hints.last().unwrap());
            v
        } else {
            f.hints[..keep].to_vec()
        }
    };
    let width_of = |keep: usize, caps: bool, word: bool| -> usize {
        let mut n = 2; // ╭●
        if word {
            n += 1 + display_width(f.word) as usize;
        }
        let hints = shown(keep);
        if !hints.is_empty() {
            n += 2 + if caps { 4 } else { 0 }; // "  " then "░ " … " ░"
            for (i, (k, l)) in hints.iter().enumerate() {
                if i > 0 {
                    n += 3; // " · "
                }
                n += display_width(k) as usize + 1 + display_width(l) as usize;
            }
        }
        n
    };
    let (mut keep, mut caps, mut word) = (f.hints.len(), true, true);
    while keep > 0 && width_of(keep, caps, word) > w {
        keep -= 1;
    }
    if width_of(keep, caps, word) > w {
        caps = false;
    }
    if width_of(keep, caps, word) > w {
        word = false;
    }
    let hints = shown(keep);

    let buf = frame.buffer_mut();
    let right = area.right();
    let mut x = area.x;
    let lamp = format!("{}{}", f.arm, f.bulb);
    put(buf, &mut x, area.y, right, &lamp, theme::lamp());
    if word {
        put(buf, &mut x, area.y, right, " ", Style::default());
        put(buf, &mut x, area.y, right, f.word, theme::wordmark());
    }
    if !hints.is_empty() {
        put(buf, &mut x, area.y, right, "  ", Style::default());
        if caps {
            put(buf, &mut x, area.y, right, "░ ", theme::dim());
        }
        for (i, (key, label)) in hints.iter().enumerate() {
            if i > 0 {
                put(buf, &mut x, area.y, right, " · ", theme::dim());
            }
            put(buf, &mut x, area.y, right, key, theme::lamp());
            put(buf, &mut x, area.y, right, " ", Style::default());
            put(buf, &mut x, area.y, right, label, theme::dim());
        }
        if caps {
            put(buf, &mut x, area.y, right, " ░", theme::dim());
        }
    }
}

// ---------------------------------------------------------------------------
// The home screen
// ---------------------------------------------------------------------------

/// Box-drawing rather than a figlet font: every glyph is single-width, so it
/// cannot misalign the way a block font can, and it matches the `│` quote bars
/// the reader already paints.
/// The lamplight splash: the good desk lamp — the one that stays on when
/// everything else moves — lighting the name. Rows 2–4 are the wall plus
/// this pool; the middle row seats the word inside it.
const SPLASH_POOL: &str = "░░░░░░░░░░░░░░░░░";
/// The desk the lamp stands on — a carrel is a desk, so the post gets one.
/// `┷` joins the light post to a heavy tabletop; the width is the splash's.
const SPLASH_DESK: &str = "┷━━━━━━━━━━━━━━━━━━";
const TAGLINE: &str = "a quiet place to read your markdown";

// Splash width (wall 2 + pool 17, measured with `display_width` and asserted
// in the tests) and the smallest terminal that earns the banner now live in
// `home.rs`, because `home::list_geometry` needs them to say where the file
// list starts — and hit-testing inverts that same function.
use crate::home::{BANNER_MIN_COLS, BANNER_MIN_ROWS, SPLASH_W};

fn draw_home(frame: &mut Frame, app: &App, home: &Home) {
    let area = frame.area();
    let banner = area.width >= BANNER_MIN_COLS && area.height >= BANNER_MIN_ROWS;
    let mut y = area.y;

    {
        let buf = frame.buffer_mut();
        if banner {
            // Three styles, all existing palette slots, so every theme
            // lights the lamp correctly: wood arm, amber glow, brand word.
            let x = area.x + 1;
            buf.set_stringn(x, y, "╭──", 3, theme::dim());
            buf.set_stringn(x + 3, y, "●", 1, theme::lamp());
            y += 1;
            for row in 0..3u16 {
                buf.set_stringn(x, y, "│ ", 2, theme::dim());
                if row == 1 {
                    buf.set_stringn(x + 2, y, "░░░░", 4, theme::lamp());
                    buf.set_stringn(x + 6, y, "  carrel  ", 10, theme::wordmark());
                    buf.set_stringn(x + 16, y, "░░░", 3, theme::lamp());
                } else {
                    buf.set_stringn(x + 2, y, SPLASH_POOL, 17, theme::lamp());
                }
                y += 1;
            }
            buf.set_stringn(x, y, SPLASH_DESK, SPLASH_W as usize, theme::dim());
            y += 1;
            buf.set_stringn(x, y, TAGLINE, area.width as usize, theme::dim());
            y += 2;
        } else {
            buf.set_stringn(area.x, y, "carrel", area.width as usize, theme::wordmark());
            y += 1;
        }

        // The active root, always visible, so the precedence rule is never a
        // mystery: explicit argument > saved root > current directory.
        let root = home.root.display().to_string();
        buf.set_stringn(area.x + 1, y, &root, area.width as usize, theme::dim());
        y += 1;
    }

    // The list rect comes from `home::list_geometry`, NOT from the `y` this
    // function happened to paint to — because `Home::row_at` inverts that same
    // function to turn a click into a file. Two derivations would drift and
    // every click would land on the wrong row.
    let (list_top, list_h) = crate::home::list_geometry(area.width, area.height, app.hints);
    debug_assert_eq!(
        y, list_top,
        "the header painted to row {y} but list_geometry says {list_top}"
    );
    let chrome = if app.hints { 2 } else { 1 };
    let list_bottom = area.bottom().saturating_sub(chrome);
    let list = Rect::new(area.x, list_top, area.width, list_h);
    if home.mode == HomeMode::Search {
        paint_hits(frame, home, list);
    } else {
        paint_entries(frame, home, list);
    }
    paint_home_status(
        frame,
        app,
        home,
        Rect::new(area.x, list_bottom, area.width, 1),
    );
    if app.hints {
        paint_footer(
            frame,
            app,
            Rect::new(area.x, list_bottom + 1, area.width, 1),
        );
    }

    if home.mode == HomeMode::Picker {
        paint_picker(frame, home, area);
    }
}

fn paint_entries(frame: &mut Frame, home: &Home, area: Rect) {
    if area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();

    if home.filtered.is_empty() {
        let msg = if home.scanning {
            "Looking…"
        } else if home.entries.is_empty() {
            "Nothing to read here. Press d to choose a directory."
        } else {
            "No file matches that filter."
        };
        buf.set_stringn(area.x + 2, area.y, msg, area.width as usize, theme::dim());
        return;
    }

    // The window comes from `home::window_first`, which `Home::row_at`
    // inverts — and which scrolls only when the selection would fall off it,
    // so a click never drags the list out from under the pointer.
    let h = area.height as usize;
    let first = crate::home::window_first(home.top, home.selected, home.filtered.len(), h);
    for (row, &idx) in home.filtered.iter().skip(first).take(h).enumerate() {
        let y = area.y + row as u16;
        let e = &home.entries[idx];
        let shown = e
            .path
            .strip_prefix(&home.root)
            .unwrap_or(&e.path)
            .display()
            .to_string();
        let is_sel = first + row == home.selected;
        let style = if is_sel {
            theme::selected()
        } else {
            Style::default()
        };
        if is_sel {
            buf.set_stringn(area.x, y, "▸ ", 2, theme::selected());
        }
        buf.set_stringn(
            area.x + 2,
            y,
            &shown,
            area.width.saturating_sub(2) as usize,
            style,
        );
    }
}

/// Content-search results: file, right-aligned count, then the first
/// matching line dimmed underneath — two rows per hit.
fn paint_hits(frame: &mut Frame, home: &Home, area: Rect) {
    if area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    if home.hits.is_empty() {
        let msg = if home.query.is_empty() {
            "Type to search inside every file here."
        } else if home.grep_done {
            "No file contains that."
        } else {
            "Searching…"
        };
        buf.set_stringn(area.x + 2, area.y, msg, area.width as usize, theme::dim());
        return;
    }
    let per = 2usize; // name row + context row
    let visible = (area.height as usize / per).max(1);
    let first =
        crate::home::window_first(home.hit_top, home.hit_selected, home.hits.len(), visible);
    for (i, hit) in home.hits.iter().skip(first).take(visible).enumerate() {
        let y = area.y + u16::try_from(i * per).unwrap_or(u16::MAX);
        let shown = hit
            .path
            .strip_prefix(&home.root)
            .unwrap_or(&hit.path)
            .display()
            .to_string();
        let is_sel = first + i == home.hit_selected;
        let style = if is_sel {
            theme::selected()
        } else {
            Style::default()
        };
        if is_sel {
            buf.set_stringn(area.x, y, "▸ ", 2, theme::selected());
        }
        let count = format!(" {:>4}", hit.count);
        let cw = u16::try_from(count.chars().count()).unwrap_or(0);
        let name_w = area.width.saturating_sub(2 + cw);
        buf.set_stringn(area.x + 2, y, &shown, name_w as usize, style);
        buf.set_stringn(
            area.right().saturating_sub(cw),
            y,
            &count,
            cw as usize,
            style,
        );
        if y + 1 < area.bottom() {
            buf.set_stringn(
                area.x + 4,
                y + 1,
                &hit.first_line,
                area.width.saturating_sub(4) as usize,
                theme::dim(),
            );
        }
    }
}

fn paint_home_status(frame: &mut Frame, app: &App, home: &Home, area: Rect) {
    let left = match home.mode {
        HomeMode::Filter => format!("filter: {}", home.filter),
        HomeMode::Normal => home.note.clone().unwrap_or_else(|| "normal".into()),
        HomeMode::Picker => "choose a directory".into(),
        HomeMode::Search => format!("search: {}", home.query),
    };
    let mut right = if home.mode == HomeMode::Search {
        let state = if home.grep_done {
            "found"
        } else {
            "searching…"
        };
        format!("{} file(s) {state}", home.hits.len())
    } else {
        format!("{} of {}", home.filtered.len(), home.entries.len())
    };
    if home.scanning {
        right.push_str("   ⟳ scanning…");
    }
    if home.unreadable > 0 {
        let _ = write!(right, "   {} unreadable", home.unreadable);
    }

    let buf = frame.buffer_mut();
    buf.set_style(area, theme::status());
    let lx = fold_lamp(buf, app.hints, area);
    buf.set_stringn(lx, area.y, &left, area.width as usize, theme::status());
    let rw = u16::try_from(right.chars().count()).unwrap_or(0);
    let rx = area.right().saturating_sub(rw);
    if rx > lx + u16::try_from(left.chars().count()).unwrap_or(0) {
        buf.set_stringn(rx, area.y, &right, rw as usize, theme::status());
    }
}

/// The breadcrumb band: the section path on row 0, a rule on row 1. Only
/// when [`App::band`] — the same predicate that reserved the rows, so paint
/// and geometry cannot disagree. The crumb aligns with the prose column
/// (`text_x`) and fits the measure; the rule spans the full text area, the
/// wide edge, because it separates chrome from content.
fn paint_breadcrumb(frame: &mut Frame, app: &App, text: Rect) {
    let Some(crumb) = crate::breadcrumb::of(app, app.text_w()) else {
        return;
    };
    let buf = frame.buffer_mut();
    let rule: String = "─".repeat(text.width as usize);
    buf.set_stringn(text.x, 1, &rule, text.width as usize, theme::dim());
    if crumb.segments.is_empty() {
        return;
    }
    let mut line = String::new();
    if crumb.elided {
        line.push_str(crate::breadcrumb::ELLIPSIS);
    }
    for (i, (_, t)) in crumb.segments.iter().enumerate() {
        if i > 0 {
            line.push_str(crate::breadcrumb::SEP);
        }
        line.push_str(t);
    }
    let x = app.text_x_now();
    buf.set_stringn(x, 0, &line, app.text_w() as usize, theme::dim());
}

fn paint_picker(frame: &mut Frame, home: &Home, area: Rect) {
    // Geometry from `Home::picker_view`, which `Home::picker_row_at` inverts
    // to turn a click into a directory. One derivation, both ways.
    let ((px, py, width, height), first, visible) = home.picker_view(area.width, area.height);
    let bx = Rect::new(area.x + px, area.y + py, width, height);
    let w = width;

    let buf = frame.buffer_mut();
    // Clear underneath so the list does not show through the overlay.
    for yy in bx.y..bx.bottom() {
        buf.set_stringn(
            bx.x,
            yy,
            " ".repeat(w as usize),
            w as usize,
            Style::default(),
        );
    }
    buf.set_stringn(bx.x, bx.y, " choose a directory", w as usize, theme::dim());

    // The input row. The path is right-anchored inside it, because what you
    // are typing is the END of a path and that is the part worth seeing.
    if bx.height > 1 {
        let inner = usize::from(w.saturating_sub(4));
        let typed: String = {
            let n = home.picker.typed.chars().count();
            home.picker
                .typed
                .chars()
                .skip(n.saturating_sub(inner))
                .collect()
        };
        buf.set_stringn(
            bx.x + 1,
            bx.y + 1,
            format!("› {typed}▏"),
            w.saturating_sub(1) as usize,
            theme::selected(),
        );
    }

    for (i, root) in home
        .picker
        .roots
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
    {
        let Ok(off) = u16::try_from(i - first) else {
            break;
        };
        let yy = bx.y + 2 + off;
        if yy >= bx.bottom() {
            break;
        }
        let sel = i == home.picker.selected;
        let style = if sel {
            theme::selected()
        } else {
            Style::default()
        };
        let text = format!("{} {}", if sel { "▸" } else { " " }, root.display());
        buf.set_stringn(bx.x + 1, yy, &text, w.saturating_sub(1) as usize, style);
    }

    // An empty list is a dead end unless it says so.
    if home.picker.roots.is_empty() && bx.height > 2 {
        buf.set_stringn(
            bx.x + 1,
            bx.y + 2,
            "  no directory matches",
            w.saturating_sub(1) as usize,
            theme::dim(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Direction, SearchKey};
    use carrel_core::Document;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;

    fn buffer_of(app: &App, cols: u16, rows: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        t.backend().buffer().clone()
    }

    #[test]
    fn a_folded_section_paints_its_marked_heading_and_none_of_its_body() {
        let src = "# Alpha\n\nalpha body\n\n# Beta\n\nbeta body\n";
        let mut app = App::new("t.md".into(), Document::parse(src), 40, 10);
        app.breadcrumb = false;
        let alpha = app
            .doc
            .nodes
            .iter()
            .find(|n| {
                matches!(n.kind, NodeKind::Heading { .. })
                    && &app.doc.text[n.doc.start as usize..n.doc.end as usize] == "Alpha"
            })
            .map(|n| n.id)
            .unwrap();
        app.folded.insert(alpha);
        app.on_resize(40, 10);
        let buf = buffer_of(&app, 40, 10);
        let all: String = (0..10).map(|y| line(&buf, y) + "\n").collect();
        assert!(all.contains("Alpha"), "{all}");
        assert!(!all.contains("alpha body"), "folded body absent: {all}");
        assert!(all.contains("beta body"), "the open section still paints");
        assert!(all.contains('▸'), "the fold marker shows: {all}");
        assert!(all.contains('…'), "and the suffix: {all}");
    }

    #[test]
    fn the_breadcrumb_band_paints_the_path_and_a_rule() {
        let src = "# Top\n\nintro\n\n## Mid\n\nbody one\n\nbody two\n\nbody three\n";
        let mut app = App::new("t.md".into(), Document::parse(src), 40, 8);
        // Scroll until "body two" is the top visible content.
        while {
            let b = app.layout.block_at_row(app.view.scroll_row);
            let n = app.doc.node_for_block(b);
            !app.doc.text[n.doc.start as usize..n.doc.end as usize].starts_with("body two")
        } {
            crate::app::update(
                &mut app,
                crate::action::Action::Scroll(crate::action::Span::Line, 1),
            );
        }
        let buf = buffer_of(&app, 40, 8);
        assert!(
            line(&buf, 0).contains("Top ▸ Mid"),
            "crumb row: {:?}",
            line(&buf, 0)
        );
        assert!(
            line(&buf, 1).contains("───"),
            "rule row: {:?}",
            line(&buf, 1)
        );
        assert!(
            !line(&buf, 1).contains("body"),
            "the rule row carries no text"
        );

        // Band off: classic geometry, blank top margin.
        app.breadcrumb = false;
        app.on_resize(40, 8);
        let buf = buffer_of(&app, 40, 8);
        assert_eq!(line(&buf, 0), "", "top margin back to blank");
    }

    fn frame_of(src: &str, cols: u16, rows: u16) -> Buffer {
        let mut app = App::new("t.md".into(), Document::parse(src), cols, rows);
        // These tests exercise text painting in classic geometry; the
        // breadcrumb band has its own tests with explicit offsets.
        app.breadcrumb = false;
        app.on_resize(cols, rows);
        buffer_of(&app, cols, rows)
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// `line`, in TEXT coordinates — the page margins cropped away.
    fn text_line(buf: &Buffer, y: u16) -> String {
        let full: String = (0..buf.area.width)
            .map(|x| buf[(x, y + crate::app::PAD_TOP)].symbol())
            .collect();
        full.chars()
            .skip(crate::app::PAD_LEFT as usize)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// A cell addressed in TEXT coordinates.
    fn tcell(buf: &Buffer, x: u16, y: u16) -> &ratatui::buffer::Cell {
        &buf[(x + crate::app::PAD_LEFT, y + crate::app::PAD_TOP)]
    }

    #[test]
    fn a_wrapped_blockquote_shows_its_bar_on_every_row() {
        let buf = frame_of("> alpha beta gamma delta epsilon\n", 16, 6);
        assert!(
            text_line(&buf, 0).starts_with('│'),
            "row 0: {:?}",
            text_line(&buf, 0)
        );
        assert!(
            text_line(&buf, 1).starts_with('│'),
            "row 1: {:?}",
            text_line(&buf, 1)
        );
    }

    #[test]
    fn a_wrapped_list_item_shows_its_marker_once_and_hangs() {
        let buf = frame_of("- alpha beta gamma delta epsilon\n", 16, 6);
        assert!(
            text_line(&buf, 0).starts_with("- "),
            "row 0: {:?}",
            text_line(&buf, 0)
        );
        let second = text_line(&buf, 1);
        assert!(second.starts_with("  "), "row 1 must hang: {second:?}");
        assert!(!second.trim_start().is_empty(), "row 1 must have text");
    }

    #[test]
    fn an_ordered_marker_past_nine_reserves_four_cells() {
        // Row 1 is the block gap; the second item sits below it.
        let buf = frame_of("9. nine\n10. ten\n", 20, 8);
        assert!(
            !text_line(&buf, 1).chars().any(char::is_alphanumeric),
            "gap row holds only the scrollbar: {:?}",
            text_line(&buf, 1),
        );
        assert!(
            text_line(&buf, 2).starts_with("10. "),
            "{:?}",
            text_line(&buf, 2)
        );
    }

    #[test]
    fn a_quote_inside_a_list_item_hangs_its_bar_at_the_item_indent() {
        let buf = frame_of(
            "- item\n\n  > quoted inside, long enough to wrap around\n",
            24,
            8,
        );
        let bar_rows: Vec<String> = (0..6)
            .map(|y| text_line(&buf, y))
            .filter(|l| l.contains('│'))
            .collect();
        assert!(!bar_rows.is_empty(), "a bar must paint");
        for l in &bar_rows {
            assert!(
                l.starts_with("  │"),
                "the bar hangs at the item's indent, not the margin: {l:?}"
            );
        }
    }

    /// The page reads like a page: text inset from every edge (field note).
    #[test]
    fn the_reader_page_has_margins_on_all_four_sides() {
        use crate::app::{PAD_LEFT, PAD_TOP};
        let buf = frame_of("hello\n", 30, 8);
        // Top margin: the first row paints no text.
        assert_eq!(line(&buf, 0), "", "top margin row: {:?}", line(&buf, 0));
        // Left margin: the first line of text starts PAD_LEFT cells in.
        let first = line(&buf, PAD_TOP);
        assert!(
            first.starts_with(&format!("{}hello", " ".repeat(PAD_LEFT as usize))),
            "left margin: {first:?}"
        );
        // Bottom margin: the row above the status bar is blank; the status
        // sits above the lamplight footer, which owns the last row.
        assert_eq!(line(&buf, 5), "", "bottom margin row: {:?}", line(&buf, 5));
        assert!(
            line(&buf, 6).contains("t.md"),
            "status: {:?}",
            line(&buf, 6)
        );
        assert!(
            line(&buf, 7).starts_with("╭●"),
            "footer: {:?}",
            line(&buf, 7)
        );
    }

    #[test]
    fn the_status_line_names_the_file() {
        let buf = frame_of("hello\n", 24, 4);
        assert!(
            line(&buf, 2).contains("t.md"),
            "status: {:?}",
            line(&buf, 2)
        );
    }

    /// A key nobody can see is a feature nobody has — the theme cycle must
    /// be advertised on screen, not only in `--help` (field note).
    #[test]
    fn the_status_line_advertises_the_theme_key() {
        let buf = frame_of("hello\n", 40, 4);
        assert!(
            line(&buf, 2).contains("T theme"),
            "status: {:?}",
            line(&buf, 2)
        );
    }

    #[test]
    fn a_viewport_too_small_to_lay_out_says_so_instead_of_panicking() {
        let buf = frame_of("hello\n", 4, 1);
        assert!(!line(&buf, 0).is_empty());
    }

    #[test]
    fn the_current_match_is_styled_differently_from_the_others() {
        let mut app = App::new("t.md".into(), Document::parse("alpha alpha"), 20, 5);
        crate::app::update(&mut app, Action::SearchOpen(Direction::Forward));
        for c in "alpha".chars() {
            crate::app::update(&mut app, Action::SearchKey(SearchKey::Char(c)));
        }
        crate::app::update(&mut app, Action::SearchKey(SearchKey::Accept));
        assert_eq!(app.matches.as_ref().unwrap().len(), 2);

        let buf = buffer_of(&app, 20, 5);
        assert_eq!(
            tcell(&buf, 0, 0).style().bg,
            theme::match_current().bg,
            "first is current"
        );
        assert_eq!(
            tcell(&buf, 6, 0).style().bg,
            theme::match_normal().bg,
            "second is not"
        );
    }

    #[test]
    fn table_columns_align_with_a_separator_and_a_bold_header() {
        let buf = frame_of("| a | b |\n|---|---|\n| one | two |\n", 30, 8);
        let joined: String = (0..6)
            .map(|y| text_line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined.contains("onetwo"), "cells collided:\n{joined}");
        // Columns pad to max-content width, so "two" starts at the same
        // column on every row, with the separator painted into the gap.
        assert!(joined.contains("a   │ b"), "header misaligned:\n{joined}");
        assert!(joined.contains("one │ two"), "row misaligned:\n{joined}");
        assert!(
            tcell(&buf, 0, 0)
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD),
            "header must be bold",
        );
        assert!(
            !tcell(&buf, 0, 2)
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD),
            "data rows must not be bold",
        );
    }

    #[test]
    fn a_match_inside_a_card_lands_on_the_value_never_on_the_gutter_label() {
        // Aligned form is `name`(5) + `description`(39) + 3 = 47 columns wide;
        // width 30 overflows it into cards. The gutter is
        // (widest header "description" = 11, + 2 = 13).min(width/3 = 10) = 10,
        // so any highlighted cell left of x=10 would be sitting on the dim
        // gutter label instead of the value text it is supposed to mark.
        const GUTTER: u16 = 10; // node.indent (0) + the gutter computed above.
        let src = "| name | description |\n|---|---|\n\
                   | alpha | a value easily long enough to overflow |\n";
        let mut app = App::new("t.md".into(), Document::parse(src), 30, 10);
        crate::app::update(&mut app, Action::SearchOpen(Direction::Forward));
        for c in "value".chars() {
            crate::app::update(&mut app, Action::SearchKey(SearchKey::Char(c)));
        }
        crate::app::update(&mut app, Action::SearchKey(SearchKey::Accept));
        assert_eq!(
            app.matches.as_ref().unwrap().len(),
            1,
            "one hit in the value cell"
        );

        let buf = buffer_of(&app, 30, 10);
        let is_match_bg = |bg| bg == theme::match_current().bg || bg == theme::match_normal().bg;

        let hits: Vec<(u16, u16)> = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| is_match_bg(buf[(x, y)].style().bg))
            .collect();
        assert!(!hits.is_empty(), "the search hit must be painted somewhere");
        for &(x, y) in &hits {
            assert!(
                x >= GUTTER,
                "match landed at x={x} y={y}, inside the gutter (< {GUTTER}); \
                 line: {:?}",
                line(&buf, y),
            );
        }
    }

    #[test]
    fn osc8_text_and_url_are_stripped_of_control_characters() {
        let app = App::new(
            "t.md".into(),
            Document::parse("[a\u{1b}b](https://e.com/x)"),
            40,
            6,
        );
        let mut t = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let mut links = Vec::new();
        t.draw(|f| draw_with_links(f, &app, &mut links)).unwrap();
        assert_eq!(links.len(), 1, "{links:?}");
        assert!(
            !links[0].text.chars().any(char::is_control),
            "text leaked a control character: {:?}",
            links[0].text,
        );
        assert!(
            !links[0].text.contains('\u{1b}'),
            "the ESC byte survived: {:?}",
            links[0].text,
        );
    }

    #[test]
    fn the_help_overlay_paints_when_open_and_not_when_closed() {
        // 30 rows: tall enough that the whole reader table fits unscrolled.
        let mut app = App::new("t.md".into(), Document::parse("body text\n"), 60, 30);
        let closed = buffer_of(&app, 60, 30);
        let closed_text: String = (0..30).map(|y| line(&closed, y) + "\n").collect();
        assert!(
            !closed_text.contains("this help"),
            "no overlay while closed"
        );

        app.help = Some(0);
        let open = buffer_of(&app, 60, 30);
        let open_text: String = (0..30).map(|y| line(&open, y) + "\n").collect();
        assert!(
            open_text.contains("carrel — keys"),
            "title painted:\n{open_text}"
        );
        assert!(open_text.contains("this help"), "a reader row painted");
        assert!(open_text.contains("motions"), "a group heading painted");
    }

    #[test]
    fn help_scroll_clamps_to_the_sheet_length() {
        let mut app = App::new("t.md".into(), Document::parse("body\n"), 60, 10);
        app.help = Some(9999); // state saturates; the painter must clamp
        let buf = buffer_of(&app, 60, 10);
        let text: String = (0..10).map(|y| line(&buf, y) + "\n").collect();
        // The table's LAST row must be visible when scrolled past the end —
        // that is what "clamped to the tail" means. Assert on the actual
        // last entry so this test tracks the table instead of guessing at
        // what happens to sit near the bottom (it broke once when the mouse
        // group grew the table).
        let (_, last_desc) = crate::keys::READER_HELP.last().unwrap();
        assert!(
            text.contains(last_desc),
            "clamped view must show the table's last row {last_desc:?}:\n{text}"
        );
    }

    #[test]
    fn the_home_help_shows_home_keys() {
        let mut app = home_app(3, 60, 20);
        app.help = Some(0);
        let buf = buffer_of(&app, 60, 20);
        let text: String = (0..20).map(|y| line(&buf, y) + "\n").collect();
        assert!(
            text.contains("directory: type, it completes"),
            "home rows painted:\n{text}"
        );
    }

    #[test]
    fn a_selection_paints_reversed_cells_and_only_those() {
        // "alpha beta" on one row; select "beta" (doc bytes 6..10).
        let mut app = App::new("t.md".into(), Document::parse("alpha beta\n"), 40, 6);
        app.selection = Some(6..10);
        let buf = buffer_of(&app, 40, 6);
        for x in 6..10u16 {
            assert!(
                tcell(&buf, x, 0)
                    .style()
                    .add_modifier
                    .contains(Modifier::REVERSED),
                "cell {x} should be reversed"
            );
        }
        for x in [0u16, 5, 10] {
            assert!(
                !tcell(&buf, x, 0)
                    .style()
                    .add_modifier
                    .contains(Modifier::REVERSED),
                "cell {x} should NOT be reversed"
            );
        }
    }

    #[test]
    fn a_selection_spanning_a_wrap_paints_on_both_rows() {
        // Width 12 → text width 7: "alpha beta" wraps after "alpha".
        let mut app = App::new("t.md".into(), Document::parse("alpha beta\n"), 12, 6);
        app.selection = Some(3..9); // "ha be" across the wrap
        let buf = buffer_of(&app, 12, 6);
        assert!(
            tcell(&buf, 3, 0)
                .style()
                .add_modifier
                .contains(Modifier::REVERSED),
            "row 0 tail selected"
        );
        assert!(
            tcell(&buf, 0, 1)
                .style()
                .add_modifier
                .contains(Modifier::REVERSED),
            "row 1 head selected"
        );
    }

    #[test]
    fn diagram_art_paints_instead_of_source_and_m_restores_it() {
        let src = "intro\n\n```mermaid\ngraph TD\n alpha-->beta\n```\n";
        let mut app = App::new("t.md".into(), Document::parse(src), 40, 12);
        app.diagram_art.insert(
            carrel_core::BlockIdx(1),
            vec!["┌ARTBOX┐".into(), "└──────┘".into()],
        );
        app.relayout();
        let buf = buffer_of(&app, 40, 12);
        let text: String = (0..12).map(|y| line(&buf, y) + "\n").collect();
        assert!(text.contains("ARTBOX"), "art painted:\n{text}");
        assert!(!text.contains("graph TD"), "source hidden:\n{text}");

        crate::app::update(&mut app, Action::RenderedToggle);
        let buf = buffer_of(&app, 40, 12);
        let text: String = (0..12).map(|y| line(&buf, y) + "\n").collect();
        assert!(!text.contains("ARTBOX"), "art hidden after m:\n{text}");
        assert!(text.contains("graph TD"), "source restored:\n{text}");
    }

    /// The panel's row span: (first row after the title, footer row). The
    /// document is still painted BEHIND the panel, so tests must scope their
    /// searches to it or they find the underlying heading text instead —
    /// which is exactly how this test failed on its first run.
    fn panel_rows(buf: &Buffer, title: &str) -> (u16, u16) {
        let top = (0..buf.area.height)
            .find(|y| line(buf, *y).contains(title))
            .unwrap_or_else(|| panic!("no {title} panel"));
        let bottom = (top + 1..buf.area.height)
            .find(|y| line(buf, *y).contains('└'))
            .unwrap_or_else(|| panic!("no panel footer"));
        (top + 1, bottom)
    }

    #[test]
    fn the_outline_panel_lists_headings_indented_by_level() {
        let src = "# One\n\nbody\n\n## Two Deep\n\nbody\n\n# Three\n\nbody\n";
        let mut app = App::new("t.md".into(), Document::parse(src), 60, 20);
        crate::app::update(&mut app, Action::OutlineToggle);
        let buf = buffer_of(&app, 60, 20);
        let (top, bottom) = panel_rows(&buf, "outline");
        let rows: Vec<String> = (top..bottom).map(|y| line(&buf, y)).collect();
        let one = rows.iter().find(|l| l.contains("One")).unwrap();
        let two = rows.iter().find(|l| l.contains("Two Deep")).unwrap();
        // Compare the LABEL columns in CHARS — the document bleeds through
        // left of the panel (so leading-space counts lie), and the ▸ marker
        // is multi-byte (so `find`'s byte offsets lie too).
        let col = |l: &str, needle: &str| l[..l.find(needle).unwrap()].chars().count();
        assert!(
            col(two, "Two Deep") > col(one, "One"),
            "level 2 indents deeper: {one:?} vs {two:?}"
        );
    }

    #[test]
    fn the_outline_selected_row_is_styled_and_filter_removes_rows() {
        let src = "# Alpha\n\nbody\n\n# Beta\n\nbody\n";
        let mut app = App::new("t.md".into(), Document::parse(src), 60, 20);
        crate::app::update(&mut app, Action::OutlineToggle);
        let buf = buffer_of(&app, 60, 20);
        let (top, bottom) = panel_rows(&buf, "outline");
        let find_y = |buf: &Buffer, needle: &str| -> u16 {
            (top..bottom)
                .find(|y| line(buf, *y).contains(needle))
                .unwrap_or_else(|| panic!("{needle} not in the panel"))
        };
        let ya = find_y(&buf, "Alpha");
        let x = u16::try_from(line(&buf, ya).find('A').unwrap()).unwrap();
        assert!(
            buf[(x, ya)].style().add_modifier.contains(Modifier::BOLD),
            "selected row styled"
        );
        let yb = find_y(&buf, "Beta");
        let xb = u16::try_from(line(&buf, yb).find('B').unwrap()).unwrap();
        assert!(
            !buf[(xb, yb)].style().add_modifier.contains(Modifier::BOLD),
            "unselected row plain"
        );

        crate::app::update(&mut app, Action::OutlineKey(SearchKey::Char('b')));
        crate::app::update(&mut app, Action::OutlineKey(SearchKey::Char('e')));
        let buf = buffer_of(&app, 60, 20);
        let (top, bottom) = panel_rows(&buf, "outline");
        let panel: String = (top..bottom).map(|y| line(&buf, y) + "\n").collect();
        assert!(!panel.contains("Alpha"), "filtered out:\n{panel}");
        assert!(panel.contains("Beta"));
    }

    #[test]
    fn wikilinks_emit_osc8_only_when_resolved_and_as_file_uris() {
        let mut app = App::new(
            "t.md".into(),
            Document::parse("see [[Known]] and [[Unknown]]\n"),
            60,
            8,
        );
        app.wiki.insert(
            carrel_core::LinkId(0),
            std::path::PathBuf::from("/tmp/Known.md"),
        );
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        let mut links = Vec::new();
        t.draw(|f| draw_with_links(f, &app, &mut links)).unwrap();
        assert_eq!(links.len(), 1, "the unresolved wikilink emits nothing");
        assert_eq!(links[0].url, "file:///tmp/Known.md");
        assert_eq!(links[0].text, "Known");
    }

    fn home_app(n: usize, cols: u16, rows: u16) -> App {
        use crate::scan::Entry;
        use std::time::{Duration, SystemTime};
        let entries = (0..n)
            .map(|i| Entry {
                path: std::path::PathBuf::from(format!("/root/file{i}.md")),
                mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(100 - i as u64),
            })
            .collect();
        App::new_home("/root".into(), entries, cols, rows)
    }

    fn buffer_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| line(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_reader_paints_the_lamplight_footer() {
        let buf = frame_of("# T\n\nbody text here", 60, 12);
        let foot = line(&buf, 11);
        assert!(foot.starts_with("╭●"), "lamp first: {foot:?}");
        assert!(
            foot.contains("reading") && foot.contains("j/k scroll"),
            "{foot:?}"
        );
        assert!(
            line(&buf, 10).contains("t.md"),
            "status sits above the footer"
        );
    }

    #[test]
    fn the_footer_is_lit_in_lamp_wordmark_and_dim() {
        let buf = frame_of("# T\n\nbody", 60, 12);
        assert_eq!(buf[(1, 11)].style().fg, theme::lamp().fg, "the bulb");
        assert_eq!(buf[(3, 11)].style().fg, theme::wordmark().fg, "the word");
    }

    #[test]
    fn hiding_folds_the_lamp_onto_the_status_row() {
        let mut app = App::new("t.md".into(), Document::parse("# T\n\nbody"), 60, 12);
        crate::app::update(&mut app, Action::HintsToggle);
        let buf = buffer_of(&app, 60, 12);
        let status = line(&buf, 11);
        assert!(status.starts_with("╰○"), "folded lamp: {status:?}");
        assert!(status.contains("t.md"));
        assert!(!buffer_text(&buf).contains("j/k scroll"), "hints are gone");
    }

    #[test]
    fn a_narrow_footer_drops_hints_from_the_right_but_keeps_h_more() {
        let buf = frame_of("# T\n\nbody", 36, 12);
        let foot = line(&buf, 11);
        assert!(
            foot.contains("h more"),
            "the door to help survives: {foot:?}"
        );
        assert!(
            !foot.contains("outline"),
            "rightmost hints dropped first: {foot:?}"
        );
        assert!(foot.contains("j/k scroll"), "leftmost hints kept: {foot:?}");
    }

    #[test]
    fn the_home_footer_replaces_the_old_hints_row() {
        let buf = buffer_of(&home_app(3, 60, 16), 60, 16);
        let foot = line(&buf, 15);
        assert!(
            foot.starts_with("╭●") && foot.contains("browse"),
            "{foot:?}"
        );
        assert!(foot.contains("d directory"), "{foot:?}");
    }

    #[test]
    fn the_search_footer_swaps_and_never_promises_n_before_accept() {
        let mut app = App::new("t.md".into(), Document::parse("# T\n\nneedle"), 60, 12);
        crate::app::update(&mut app, Action::SearchOpen(Direction::Forward));
        let buf = buffer_of(&app, 60, 12);
        let foot = line(&buf, 11);
        assert!(
            foot.contains("searching") && foot.contains("enter jump"),
            "{foot:?}"
        );
        assert!(
            !foot.contains("n/N"),
            "n/N is not live while typing: {foot:?}"
        );
    }

    #[test]
    fn the_home_screen_shows_the_lamplight_splash_when_there_is_room() {
        let buf = buffer_of(&home_app(3, 80, 24), 80, 24);
        assert!(line(&buf, 0).contains('●'), "lamp: {:?}", line(&buf, 0));
        assert!(
            line(&buf, 2).contains("carrel") && line(&buf, 2).contains('░'),
            "the word sits lit in the pool: {:?}",
            line(&buf, 2)
        );
        assert!(
            line(&buf, 4).contains('┷') && line(&buf, 4).contains('━'),
            "the desk under the lamp: {:?}",
            line(&buf, 4)
        );
        assert!(
            line(&buf, 5).contains("quiet place"),
            "row 5: {:?}",
            line(&buf, 5)
        );
    }

    #[test]
    fn the_splash_is_lit_in_the_lamp_and_wordmark_styles() {
        let buf = buffer_of(&home_app(3, 80, 24), 80, 24);
        // Art starts at x = 1: `╭──●` puts the bulb at x = 4; row 2 is
        // `│ ░░░░  carrel  ░░░` — pool from x = 3, the word's `c` at x = 9.
        assert_eq!(buf[(4, 0)].style().fg, theme::lamp().fg, "the bulb");
        assert_eq!(buf[(3, 2)].style().fg, theme::lamp().fg, "the pool");
        assert_eq!(buf[(9, 2)].style().fg, theme::wordmark().fg, "the word");
        assert_eq!(buf[(1, 1)].style().fg, theme::dim().fg, "the wall");
        assert_eq!(buf[(1, 4)].style().fg, theme::dim().fg, "the desk is wood");
    }

    #[test]
    fn a_small_terminal_collapses_the_wordmark_to_one_line() {
        let buf = buffer_of(&home_app(3, 20, 10), 20, 10);
        assert!(line(&buf, 0).contains("carrel"), "{:?}", line(&buf, 0));
        assert!(
            !line(&buf, 0).contains('░') && !line(&buf, 0).contains('●'),
            "must not paint art it cannot fit"
        );
    }

    #[test]
    fn the_splash_never_exceeds_its_measured_width() {
        assert_eq!(
            carrel_core::display_width(SPLASH_POOL) + 2,
            SPLASH_W,
            "wall + pool is the splash's full width"
        );
        assert_eq!(
            carrel_core::display_width(SPLASH_DESK),
            SPLASH_W,
            "the desk spans exactly the splash width"
        );
    }

    #[test]
    fn an_empty_root_says_so_and_points_at_the_picker() {
        let mut app = home_app(0, 60, 16);
        if let Screen::Home(h) = &mut app.screen {
            h.finish_scan(0);
        }
        let buf = buffer_of(&app, 60, 16);
        let all: String = (0..16)
            .map(|y| line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("Nothing to read here"), "{all}");
    }

    #[test]
    fn exactly_one_row_is_marked_selected() {
        let buf = buffer_of(&home_app(4, 60, 16), 60, 16);
        let marked = (0..16).filter(|y| line(&buf, *y).starts_with('▸')).count();
        assert_eq!(marked, 1);
    }

    #[test]
    fn a_live_scan_shows_an_indicator() {
        let buf = buffer_of(&home_app(2, 60, 16), 60, 16);
        let all: String = (0..16)
            .map(|y| line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("scanning"), "{all}");
    }

    #[test]
    fn the_picker_overlays_the_list() {
        let mut app = home_app(4, 60, 20);
        crate::app::update(&mut app, Action::PickerOpen);
        crate::app::update(&mut app, Action::HomeKey(SearchKey::Char('/')));
        let buf = buffer_of(&app, 60, 20);
        let all: String = (0..20)
            .map(|y| line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("choose a directory"), "{all}");
        // The input row echoes what is being typed, with a cursor after it.
        assert!(all.contains("› /▏"), "no input row:\n{all}");
        // …and the matches for it are listed beneath, `/` being one
        // directory every machine has.
        assert!(all.contains("▸ /"), "no match list:\n{all}");
    }

    #[test]
    fn a_wrapped_code_line_is_marked_under_its_own_indentation() {
        let src = "```rust\nfn main() {\n    let result = compute(alpha, beta);\n}\n```\n";
        let buf = frame_of(src, 30, 12);
        let all: String = (0..9)
            .map(|y| text_line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains('↳'), "no continuation marker:\n{all}");
        // The marker sits at the code line's own indent, column 4.
        let marked = (0..9)
            .map(|y| text_line(&buf, y))
            .find(|l| l.contains('↳'))
            .unwrap();
        assert!(marked.starts_with("    ↳ "), "marker column: {marked:?}");
    }

    #[test]
    fn wrapped_prose_gets_no_marker() {
        let buf = frame_of("alpha beta gamma delta epsilon zeta eta\n", 14, 10);
        let all: String = (0..10)
            .map(|y| line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!all.contains('↳'), "prose must not be marked:\n{all}");
    }

    #[test]
    fn visible_links_are_collected_for_the_osc8_pass() {
        let app = App::new(
            "t.md".into(),
            Document::parse("see [docs](https://example.com/d) here"),
            40,
            6,
        );
        let mut t = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let mut links = Vec::new();
        t.draw(|f| draw_with_links(f, &app, &mut links)).unwrap();
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].text, "docs");
        assert_eq!(links[0].url, "https://example.com/d");
    }

    #[test]
    fn the_selected_link_is_highlighted_not_collected() {
        let mut app = App::new(
            "t.md".into(),
            Document::parse("see [docs](https://example.com/d) here"),
            40,
            6,
        );
        crate::app::update(&mut app, Action::LinkStep(1));
        let mut t = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let mut links = Vec::new();
        t.draw(|f| draw_with_links(f, &app, &mut links)).unwrap();
        assert!(
            links.is_empty(),
            "selected link keeps its highlight instead"
        );
        let buf = t.backend().buffer().clone();
        assert_eq!(
            tcell(&buf, 4, 0).style().bg,
            theme::link_selected().bg,
            "{:?}",
            text_line(&buf, 0)
        );
        // And the URL is in the status bar for copying.
        assert!(line(&buf, 4).contains("example.com"), "{:?}", line(&buf, 4));
    }

    #[test]
    fn a_ready_image_paints_halfblock_cells_with_its_colour() {
        use ratatui_image::FontSize;
        use ratatui_image::picker::Picker;

        let mut app = App::new(
            "t.md".into(),
            Document::parse("![alt](p.png)\n\nprose after\n"),
            30,
            10,
        );
        // Pixels "arrived": a solid red 40×32 image at an 8×16 font.
        let img_block = carrel_core::BlockIdx(0);
        app.image_dims.insert(img_block, (40, 32));
        app.relayout();
        assert_eq!(
            app.layout.content_height(&app.doc, img_block),
            2,
            "32 px / 16 px per row",
        );

        // Deprecated in favour of the stdio-query constructor, which leaks an
        // input-stealing thread on timeout — the same reason main.rs avoids it.
        #[allow(deprecated)]
        let picker = Picker::from_fontsize(FontSize::new(8, 16));
        let red = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            40,
            32,
            image::Rgb([200, 30, 30]),
        ));
        let mut protocols = HashMap::from([(img_block, picker.new_resize_protocol(red))]);

        let mut t = Terminal::new(TestBackend::new(30, 10)).unwrap();
        t.draw(|f| draw_full(f, &app, &mut Vec::new(), &mut protocols))
            .unwrap();
        let buf = t.backend().buffer().clone();

        // Half-blocks carry the image colour in fg and/or bg of ▀ cells.
        let cell = tcell(&buf, 0, 0);
        let reddish = |c: Option<ratatui::style::Color>| matches!(c, Some(ratatui::style::Color::Rgb(r, g, b)) if r > 150 && g < 90 && b < 90);
        assert!(
            reddish(cell.style().fg) || reddish(cell.style().bg),
            "expected the image's red in cell (0,0): {cell:?}",
        );
        // The alt text is NOT painted for a ready image.
        assert!(
            !text_line(&buf, 0).contains("alt"),
            "{:?}",
            text_line(&buf, 0)
        );
        // Prose after the image still renders.
        let all: String = (0..10)
            .map(|y| line(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("prose after"), "{all}");
    }

    #[test]
    fn a_loading_or_remote_image_renders_dim_alt_text() {
        let buf = frame_of("![the alt text](https://example.com/x.png)\n", 40, 6);
        assert!(
            text_line(&buf, 0).contains("the alt text"),
            "{:?}",
            text_line(&buf, 0)
        );
        assert_eq!(
            tcell(&buf, 0, 0).style().fg,
            theme::dim().fg,
            "alt text is dim"
        );
    }

    #[test]
    fn at_the_bottom_the_app_says_it_is_at_the_bottom() {
        // The original report was "can't scroll to the bottom" — the motion
        // was fine, but the percent read 84% and the thumb stopped short, so
        // the reader was being told they weren't there. Both must agree now.
        let mut src = String::new();
        for i in 1..=30 {
            use std::fmt::Write as _;
            let _ = write!(src, "para {i}\n\n");
        }
        let mut app = App::new("t.md".into(), Document::parse(&src), 30, 10);
        crate::app::update(&mut app, Action::GoToEnd);

        let mut t = Terminal::new(TestBackend::new(30, 10)).unwrap();
        t.draw(|f| draw(f, &app)).unwrap();
        let buf = t.backend().buffer().clone();

        assert!(
            line(&buf, 8).contains("100%"),
            "status: {:?}",
            line(&buf, 8)
        );
        // The thumb's bottom cell is the bar's bottom cell. The bar keeps
        // the true right edge and its track aligns with the text rows.
        let bar_x = 30 - 1;
        let th = App::text_size(30, 10, true, app.band(), crate::config::DEFAULT_MEASURE).2;
        let (top_y, bottom_y) = (app.text_y(), app.text_y() + th - 1);
        assert_eq!(
            buf[(bar_x, bottom_y)].symbol(),
            "█",
            "thumb touches the bottom"
        );
        assert_eq!(buf[(bar_x, top_y)].symbol(), "│", "and has left the top");
    }

    #[test]
    fn a_rust_keyword_is_painted_with_the_keyword_style() {
        let buf = frame_of("```rust\nfn main() {}\n```\n\nprose\n", 30, 8);
        // Row 0 starts with "fn" — the keyword's own colour, not code-plain.
        assert_eq!(
            tcell(&buf, 0, 0).style().fg,
            theme::token(carrel_core::TokenKind::Keyword).fg,
            "keyword cell: {:?}",
            text_line(&buf, 0),
        );
        // The prose row keeps the terminal's own foreground.
        let prose_y = (0..5)
            .find(|y| text_line(&buf, *y).starts_with("prose"))
            .unwrap();
        assert_eq!(
            tcell(&buf, 0, prose_y).style().fg,
            Some(ratatui::style::Color::Reset),
            "prose must not inherit code styling",
        );
    }

    #[test]
    fn a_heading_is_styled_and_body_text_is_not() {
        let buf = frame_of("# Title\n\nbody\n", 20, 6);
        assert_eq!(
            tcell(&buf, 0, 0).style().fg,
            theme::heading(1).fg,
            "heading coloured"
        );
        // A cell spells "inherit the terminal" as `Color::Reset`, where a
        // `Style` spells it `None`. Both mean the palette stayed off body text.
        assert_eq!(
            tcell(&buf, 0, 1).style().fg,
            Some(ratatui::style::Color::Reset),
            "body inherits the terminal",
        );
    }

    const WIDE_MD: &str = "\
| name | city |\n|---|---|\n\
| alpha person with a long value | lisbon |\n\
| beta | kuala lumpur |\n";

    #[test]
    fn a_wide_table_paints_cards_with_labels_rules_and_a_bold_legend() {
        let buf = frame_of(WIDE_MD, 28, 16);
        let all: Vec<String> = (0..13).map(|y| text_line(&buf, y)).collect();
        // Legend first, then a rule, then labelled values.
        assert!(all[0].starts_with("name"), "legend: {:?}", all[0]);
        assert!(all.iter().any(|l| l.starts_with('─')), "rules: {all:?}");
        assert!(
            all.iter()
                .any(|l| l.starts_with("name") && l.contains("alpha")),
            "gutter label beside value: {all:?}"
        );
        assert!(
            all.iter()
                .any(|l| l.starts_with("city") && l.contains("lisbon")),
            "{all:?}"
        );
        // The legend is bold; gutter labels are not part of the text (dim).
        assert!(
            tcell(&buf, 0, 0)
                .style()
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_fitting_table_still_paints_aligned_columns_with_separators() {
        let buf = frame_of("| a | b |\n|---|---|\n| c | d |\n", 40, 6);
        assert!(text_line(&buf, 0).contains('│'), "{:?}", text_line(&buf, 0));
    }

    #[test]
    fn card_value_continuation_rows_hang_under_the_value_column() {
        let buf = frame_of(WIDE_MD, 28, 16);
        let label_row = (0..13)
            .find(|&y| {
                text_line(&buf, y).starts_with("name") && text_line(&buf, y).contains("alpha")
            })
            .unwrap();
        let cont = text_line(&buf, label_row + 1);
        assert!(
            cont.starts_with(' ') && !cont.trim().is_empty(),
            "wrapped value continues under the value column: {cont:?}"
        );
    }
}
