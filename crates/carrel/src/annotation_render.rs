//! Notes overlay; all annotations remain in doc space in the state module.
use crate::{
    action::{Action, NoteKey, Targets, Zone},
    app::App,
    theme,
};
use ratatui::{Frame, layout::Rect, style::Modifier};
use unicode_segmentation::UnicodeSegmentation;

const Z: u8 = 3;

fn button(
    frame: &mut Frame,
    targets: &mut Targets,
    x: &mut u16,
    y: u16,
    right: u16,
    label: &str,
    action: Action,
) {
    let width = carrel_core::display_width(label);
    if x.saturating_add(width) > right {
        return;
    }
    frame
        .buffer_mut()
        .set_stringn(*x, y, label, usize::from(width), theme::marker());
    targets.push(action, Zone::new(*x, y, width, 1), Z);
    *x += width + 1;
}

/// Visual rows of a draft, derived each paint; offsets are UTF-8 bytes.
fn draft_rows(text: &str, width: u16) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let (mut start, mut col) = (0, 0u16);
    for (at, grapheme) in text.grapheme_indices(true) {
        if grapheme == "\n" {
            rows.push((start, at));
            start = at + 1;
            col = 0;
            continue;
        }
        let w = carrel_core::cluster_width(grapheme);
        if col > 0 && col.saturating_add(w) > width {
            rows.push((start, at));
            start = at;
            col = 0;
        }
        col = col.saturating_add(w);
    }
    rows.push((start, text.len()));
    if col >= width {
        rows.push((text.len(), text.len()));
    }
    rows
}

#[allow(clippy::too_many_lines)]
pub fn paint(frame: &mut Frame, app: &App, targets: &mut Targets) {
    if app.notes.pane.is_none() && app.notes.draft.is_none() {
        return;
    }
    let full = frame.area();
    // The modal surface covers the reader, including its footer and image
    // payload cells; no stale document chrome should peek through the margins.
    let background = " ".repeat(usize::from(full.width));
    for y in full.y..full.bottom() {
        frame.buffer_mut().set_stringn(
            full.x,
            y,
            &background,
            usize::from(full.width),
            theme::body(),
        );
    }
    // Own the whole pointer surface, including margins outside the pane.
    targets.push(
        Action::Absorb,
        Zone::new(full.x, full.y, full.width, full.height),
        Z,
    );
    let area = if full.width >= 30 && full.height >= 8 {
        Rect::new(full.x + 2, full.y + 1, full.width - 4, full.height - 2)
    } else {
        full
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let blank = " ".repeat(usize::from(area.width));
    for y in area.y..area.bottom() {
        frame
            .buffer_mut()
            .set_stringn(area.x, y, &blank, usize::from(area.width), theme::status());
    }
    let title = if app.notes.draft.is_some() {
        "Note · Enter save · Esc cancel"
    } else {
        "Notes · ? unresolved · Esc close"
    };
    frame.buffer_mut().set_stringn(
        area.x,
        area.y,
        title,
        usize::from(area.width),
        theme::status(),
    );
    if area.height < 5 || area.width < 12 {
        if area.height > 1 {
            frame.buffer_mut().set_stringn(
                area.x,
                area.y + 1,
                "Enlarge to read notes",
                usize::from(area.width),
                theme::dim(),
            );
        }
        let mut x = area.x;
        button(
            frame,
            targets,
            &mut x,
            area.bottom() - 1,
            area.right(),
            "[close]",
            Action::Dismiss,
        );
        return;
    }
    let x = area.x + 1;
    let width = area.width - 2;
    let foot = area.bottom() - 1;
    let message = app.notes.load_error.as_ref().or(app.note.as_ref());
    let message_rows = message.map(|m| draft_rows(m, width)).unwrap_or_default();
    let message_count = message_rows
        .len()
        .min(usize::from(area.height.saturating_sub(4) / 2))
        .max(1);
    let message_top = foot.saturating_sub(u16::try_from(message_count).unwrap_or(1));
    if let Some(draft) = &app.notes.draft {
        let quote = draft.annotation.quote.replace('\n', " ");
        frame.buffer_mut().set_stringn(
            x,
            area.y + 1,
            format!("> {quote}"),
            usize::from(width),
            theme::dim(),
        );
        let rows = draft_rows(&draft.text, width);
        let cursor_row = rows
            .iter()
            .rposition(|&(start, _)| start <= draft.cursor)
            .unwrap_or(0);
        let height = usize::from(message_top.saturating_sub(area.y + 2)).max(1);
        let first = cursor_row.saturating_sub(height.saturating_sub(1));
        for (i, &(start, end)) in rows.iter().enumerate().skip(first).take(height) {
            let y = area.y + 2 + u16::try_from(i - first).unwrap_or(u16::MAX);
            frame.buffer_mut().set_stringn(
                x,
                y,
                &draft.text[start..end],
                usize::from(width),
                theme::status(),
            );
            if i == cursor_row {
                let col = carrel_core::display_width(&draft.text[start..draft.cursor.min(end)])
                    .min(width - 1);
                frame.buffer_mut().set_style(
                    Rect::new(x + col, y, 1, 1),
                    theme::selected().add_modifier(Modifier::REVERSED),
                );
            }
        }
        let mut bx = x;
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[save]",
            Action::NoteInput(NoteKey::Save),
        );
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[cancel]",
            Action::NoteInput(NoteKey::Cancel),
        );
        if bx < area.right() {
            frame.buffer_mut().set_stringn(
                bx,
                foot,
                "Ctrl-J newline",
                usize::from(area.right() - bx),
                theme::dim(),
            );
        }
    } else {
        let selected = app.notes.pane.unwrap_or(0);
        let slots = usize::from(message_top.saturating_sub(area.y + 1)) / 2;
        let slots = slots.max(1);
        let first = selected.saturating_sub(slots - 1);
        if app.notes.entries.is_empty() {
            frame.buffer_mut().set_stringn(
                x,
                area.y + 2,
                "No notes yet. Select text, then a to add a note.",
                usize::from(width),
                theme::dim(),
            );
        }
        for (i, entry) in app.notes.entries.iter().enumerate().skip(first).take(slots) {
            let y = area.y + 1 + u16::try_from((i - first) * 2).unwrap_or(u16::MAX);
            let style = if i == selected {
                theme::selected()
            } else {
                theme::status()
            };
            let marker = if entry.range.is_none() { "? " } else { "> " };
            frame.buffer_mut().set_stringn(
                x,
                y,
                format!("{marker}{}", entry.quote.replace('\n', " ")),
                usize::from(width),
                style,
            );
            let text = if entry.note.is_empty() {
                "(highlight)".into()
            } else {
                entry.note.replace('\n', " · ")
            };
            frame
                .buffer_mut()
                .set_stringn(x, y + 1, text, usize::from(width), style);
            targets.push(Action::NoteSelect(i as u32), Zone::new(x, y, width, 2), Z);
        }
        let mut bx = x;
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[go]",
            Action::NoteJump,
        );
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[edit]",
            Action::NoteEdit,
        );
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[delete]",
            Action::NoteDelete,
        );
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[export]",
            Action::NotesExport,
        );
        button(
            frame,
            targets,
            &mut bx,
            foot,
            area.right(),
            "[close]",
            Action::NotesToggle,
        );
    }
    // Wrap successful export paths so the complete destination is readable.
    if let Some(message) = message {
        for (i, &(start, end)) in message_rows.iter().take(message_count).enumerate() {
            frame.buffer_mut().set_stringn(
                x,
                message_top + u16::try_from(i).unwrap_or(0),
                &message[start..end],
                usize::from(width),
                theme::dim(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn draft_wrap_preserves_unicode_boundaries_and_final_caret_row() {
        let text = "界a\ne\u{301}🙂";
        let rows = draft_rows(text, 3);
        assert_eq!(
            rows.iter().map(|&(a, b)| &text[a..b]).collect::<Vec<_>>(),
            vec!["界a", "e\u{301}🙂", ""]
        );
    }
}
