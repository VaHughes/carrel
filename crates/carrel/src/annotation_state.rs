//! Reader annotations and their modal state. No terminal types or source writes.
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    action::{Action, NoteKey, Where},
    app::{App, Outcome},
    marginalia::{self, Annotation},
};

#[derive(Debug, Default)]
pub struct Notes {
    pub entries: Vec<Annotation>,
    pub pane: Option<usize>,
    /// Note traversal has its own byte-independent index, like search matches:
    /// a viewport anchor can stay on the same row for several different notes.
    pub current: Option<usize>,
    pub walk_anchor: Option<u32>,
    pub draft: Option<Draft>,
    /// A failed read must never turn into permission to overwrite the sidecar.
    pub load_error: Option<String>,
}

#[derive(Debug)]
pub struct Draft {
    pub entry: Option<usize>,
    pub annotation: Annotation,
    pub text: String,
    /// UTF-8 byte offset, always at a grapheme boundary.
    pub cursor: usize,
}

pub fn load(app: &mut App) {
    app.notes = Notes::default();
    if let (Some(dir), Some(file)) = (app.state_dir.as_deref(), app.file.as_deref()) {
        match marginalia::load_in(dir, file, &app.doc.text) {
            Ok(entries) => app.notes.entries = entries,
            Err(error) => {
                let message = format!("Could not read notes: {error}");
                app.notes.load_error = Some(message.clone());
                app.note = Some(message);
            }
        }
    }
}

pub fn reanchor(app: &mut App) {
    app.notes.current = None;
    app.notes.walk_anchor = None;
    for entry in &mut app.notes.entries {
        entry.reanchor(&app.doc.text);
    }
    if let Some(draft) = &mut app.notes.draft {
        draft.annotation.reanchor(&app.doc.text);
    }
    if let Some(selected) = &mut app.notes.pane {
        *selected = (*selected).min(app.notes.entries.len().saturating_sub(1));
    }
}

fn target(app: &App) -> Option<Range<u32>> {
    if app.doc.block_count() == 0 {
        return None;
    }
    let range = app.selection.clone().unwrap_or_else(|| {
        let b = app.doc.block_at_doc(carrel_core::DocByte(app.view.anchor));
        app.doc.node_for_block(b).doc.clone()
    });
    let text = app.doc.text.get(range.start as usize..range.end as usize)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let start = range.start + (text.len() - text.trim_start().len()) as u32;
    Some(start..start + trimmed.len() as u32)
}

fn selected_entry(app: &App) -> Option<usize> {
    if let Some(i) = app.notes.pane {
        return (i < app.notes.entries.len()).then_some(i);
    }
    if app.selection.is_none()
        && app.notes.walk_anchor == Some(app.view.anchor)
        && let Some(i) = app.notes.current.filter(|&i| i < app.notes.entries.len())
    {
        return Some(i);
    }
    let range = target(app)?;
    app.notes.entries.iter().position(|a| {
        a.range
            .as_ref()
            .is_some_and(|r| r.start < range.end && range.start < r.end)
    })
}

fn commit(app: &mut App, entries: Vec<Annotation>) -> bool {
    if let Some(error) = &app.notes.load_error {
        app.note = Some(error.clone());
        return false;
    }
    if let (Some(dir), Some(file)) = (app.state_dir.as_deref(), app.file.as_deref()) {
        if let Err(error) = marginalia::save_in(dir, file, &entries) {
            app.note = Some(format!("Could not save notes: {error}"));
            return false;
        }
        app.note = Some("Notes saved separately from the document.".into());
    } else {
        app.note = Some("Notes kept for this session only — export before leaving.".into());
    }
    app.notes.entries = entries;
    app.notes.current = None;
    true
}

fn begin_edit(app: &mut App) {
    if let Some(error) = &app.notes.load_error {
        app.note = Some(error.clone());
        return;
    }
    let entry = selected_entry(app);
    let annotation = entry
        .map(|i| app.notes.entries[i].clone())
        .or_else(|| Annotation::new(&app.doc.text, target(app)?, String::new()));
    let Some(annotation) = annotation else {
        app.note = Some("Select text or scroll to a paragraph to add a note.".into());
        return;
    };
    let text = annotation.note.clone();
    app.notes.draft = Some(Draft {
        entry,
        annotation,
        cursor: text.len(),
        text,
    });
    app.auto_read = false;
}

fn edit(draft: &mut Draft, key: NoteKey) {
    let left = draft.text[..draft.cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(i, _)| i);
    let right = draft.text[draft.cursor..]
        .graphemes(true)
        .next()
        .map_or(draft.cursor, |g| draft.cursor + g.len());
    match key {
        NoteKey::Char(c) if !c.is_control() => {
            draft.text.insert(draft.cursor, c);
            draft.cursor += c.len_utf8();
        }
        NoteKey::Newline => {
            draft.text.insert(draft.cursor, '\n');
            draft.cursor += 1;
        }
        NoteKey::Backspace => {
            draft.text.drain(left..draft.cursor);
            draft.cursor = left;
        }
        NoteKey::Delete => {
            draft.text.drain(draft.cursor..right);
        }
        NoteKey::Left => draft.cursor = left,
        NoteKey::Right => draft.cursor = right,
        NoteKey::Home => draft.cursor = 0,
        NoteKey::End => draft.cursor = draft.text.len(),
        _ => {}
    }
    // Inserting before a combining suffix can join two old graphemes.
    // Move to the end of the newly formed one, never leave a split cursor.
    draft.cursor = draft
        .text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i >= draft.cursor)
        .unwrap_or(draft.text.len());
}

/// Insert a bracketed paste as text, never as reader commands.
pub fn paste(app: &mut App, text: &str) {
    let Some(draft) = app.notes.draft.as_mut() else {
        return;
    };
    let clean: String = text
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .replace('\t', "    ");
    if draft.text.len().saturating_add(clean.len()) > 64 * 1024 {
        app.note = Some("That paste would make the note too large (64 KiB limit).".into());
        return;
    }
    draft.text.insert_str(draft.cursor, &clean);
    draft.cursor += clean.len();
    draft.cursor = draft
        .text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i >= draft.cursor)
        .unwrap_or(draft.text.len());
}

fn export(app: &mut App) {
    if app.notes.entries.is_empty() {
        app.note = Some("There are no notes or highlights to export.".into());
        return;
    }
    if let (Some(dir), Some(file)) = (app.state_dir.as_deref(), app.file.as_deref()) {
        match marginalia::export_in(dir, file, &app.notes.entries) {
            Ok(path) => app.note = Some(format!("Notes exported to {}", path.display())),
            Err(error) => app.note = Some(format!("Could not export notes: {error}")),
        }
    } else {
        let text = marginalia::export_markdown(std::path::Path::new(&app.path), &app.notes.entries);
        if text.len() > crate::app::CLIPBOARD_MAX {
            app.note = Some("Notes are too large for the terminal clipboard.".into());
        } else {
            app.clipboard = Some(text);
            app.note = Some("Markdown notes sent to the terminal clipboard (OSC 52).".into());
        }
    }
}

/// Called before reader actions; while the pane or editor is up nothing leaks through.
#[allow(clippy::too_many_lines)]
pub fn update(app: &mut App, action: Action) -> Option<Outcome> {
    if app.notes.draft.is_some() {
        match action {
            Action::NoteInput(NoteKey::Save) => {
                let draft = app.notes.draft.as_ref().expect("draft checked");
                let mut entry = draft.annotation.clone();
                entry.note.clone_from(&draft.text);
                let mut entries = app.notes.entries.clone();
                if let Some(i) = draft.entry {
                    entries[i] = entry;
                } else {
                    entries.push(entry);
                }
                if commit(app, entries) {
                    app.notes.draft = None;
                }
            }
            Action::NoteInput(NoteKey::Cancel) | Action::Dismiss => app.notes.draft = None,
            Action::NoteInput(key) => {
                if let Some(draft) = app.notes.draft.as_mut() {
                    edit(draft, key);
                }
            }
            Action::Quit => return Some(Outcome::Quit),
            _ => {}
        }
        return Some(Outcome::Redraw);
    }
    if app.is_home() {
        return None;
    }
    match action {
        Action::HighlightAt(byte) | Action::NoteAt(byte) => {
            if app.doc.block_count() == 0 || byte as usize >= app.doc.text.len() {
                return Some(Outcome::Idle);
            }
            let old = app.selection.clone();
            if old.as_ref().is_none_or(|r| !r.contains(&byte)) {
                let block = app.doc.block_at_doc(carrel_core::DocByte(byte));
                app.selection = Some(app.doc.node_for_block(block).doc.clone());
            }
            let action = if matches!(action, Action::HighlightAt(_)) {
                Action::HighlightAdd
            } else {
                Action::NoteEdit
            };
            update(app, action);
            app.selection = old;
        }
        Action::HighlightAdd => {
            if let Some(range) = target(app) {
                if app
                    .notes
                    .entries
                    .iter()
                    .any(|a| a.range.as_ref() == Some(&range))
                {
                    app.note = Some(
                        "That text already has a highlight. Use Add note to write about it.".into(),
                    );
                } else if let Some(entry) = Annotation::new(&app.doc.text, range, String::new()) {
                    let mut entries = app.notes.entries.clone();
                    entries.push(entry);
                    commit(app, entries);
                }
            } else {
                app.note = Some("Select text or scroll to a paragraph to highlight it.".into());
            }
        }
        Action::NoteEdit => begin_edit(app),
        Action::NotesToggle => {
            app.notes.pane = if app.notes.pane.is_some() {
                None
            } else {
                Some(app.notes.current.unwrap_or(0))
            };
            app.auto_read = false;
        }
        Action::NoteMove(delta) if app.notes.pane.is_some() => {
            let next =
                i64::try_from(app.notes.pane.unwrap_or(0)).unwrap_or(i64::MAX) + i64::from(delta);
            app.notes.pane = Some(
                usize::try_from(next.max(0))
                    .unwrap_or(usize::MAX)
                    .min(app.notes.entries.len().saturating_sub(1)),
            );
        }
        Action::NoteSelect(i) if app.notes.pane.is_some() => {
            if (i as usize) < app.notes.entries.len() {
                app.notes.pane = Some(i as usize);
            }
        }
        Action::NoteJump if app.notes.pane.is_some() => {
            if let Some(i) = selected_entry(app) {
                if let Some(range) = app.notes.entries[i].range.clone() {
                    app.notes.pane = None;
                    app.reveal_byte(range.start, app.text_h(), Where::Top);
                    app.notes.current = Some(i);
                    app.notes.walk_anchor = Some(app.view.anchor);
                } else {
                    app.note = Some(
                        "This quote is missing or ambiguous after an edit; the note is preserved."
                            .into(),
                    );
                }
            }
        }
        Action::NoteNext => {
            let mut attached: Vec<_> = app
                .notes
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, a)| a.range.as_ref().map(|r| (i, r.start)))
                .collect();
            attached.sort_by_key(|&(i, start)| (start, i));
            let previous = app
                .notes
                .current
                .filter(|_| app.notes.walk_anchor == Some(app.view.anchor));
            let next = previous
                .and_then(|p| attached.iter().position(|&(i, _)| i == p))
                .map(|i| (i + 1) % attached.len())
                .or_else(|| {
                    attached
                        .iter()
                        .position(|&(_, start)| start > app.view.anchor)
                })
                .unwrap_or(0);
            if let Some(&(index, byte)) = attached.get(next) {
                app.reveal_byte(byte, app.text_h(), Where::Top);
                app.notes.current = Some(index);
                app.note = Some(format!(
                    "Note {}/{} — {}",
                    next + 1,
                    attached.len(),
                    if app.notes.entries[index].note.is_empty() {
                        "highlight"
                    } else {
                        &app.notes.entries[index].note
                    }
                ));
                app.notes.walk_anchor = Some(app.view.anchor);
            } else {
                app.note = Some("No attached notes or highlights in this document.".into());
            }
        }
        Action::NoteDelete if app.notes.pane.is_some() => {
            if let Some(i) = selected_entry(app) {
                let mut entries = app.notes.entries.clone();
                entries.remove(i);
                if commit(app, entries) {
                    app.notes.pane = Some(i.min(app.notes.entries.len().saturating_sub(1)));
                }
            }
        }
        Action::NotesExport => export(app),
        Action::Dismiss | Action::CloseFile if app.notes.pane.is_some() => app.notes.pane = None,
        Action::Quit if app.notes.pane.is_some() => return Some(Outcome::Quit),
        _ if app.notes.pane.is_some() => return Some(Outcome::Idle),
        _ => return None,
    }
    Some(Outcome::Redraw)
}
