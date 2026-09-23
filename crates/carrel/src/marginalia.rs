//! Reader annotations anchored in display-text bytes, stored apart from documents.
//!
//! Callers inject the state directory. Missing files mean no annotations; malformed
//! records and I/O failures are errors so valuable notes are never silently lost.

use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::state::{escape_field, unescape_field, write_atomic};

const CONTEXT: usize = 32;
const HEADER: &str = "carrel-marginalia-v1";

/// A highlight, optionally accompanied by a note. An unresolved highlight keeps
/// its quote and context so later document revisions can recover its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub range: Option<Range<u32>>,
    pub quote: String,
    pub note: String,
    pub before: String,
    pub after: String,
}

impl Annotation {
    /// Capture a nonempty UTF-8-boundary range from flattened display text.
    #[must_use]
    pub fn new(text: &str, range: Range<u32>, note: String) -> Option<Self> {
        let start = range.start as usize;
        let end = range.end as usize;
        let quote = text.get(start..end).filter(|s| !s.is_empty())?;
        let before: String = text[..start].chars().rev().take(CONTEXT).collect();
        Some(Self {
            range: Some(range),
            quote: quote.into(),
            note,
            before: before.chars().rev().collect(),
            after: text[end..].chars().take(CONTEXT).collect(),
        })
    }

    /// Find the quote again after an edit. A unique exact quote is sufficient;
    /// repeated quotes require both saved contexts to match exactly. The former
    /// byte offset is never used to guess between otherwise identical passages.
    pub fn reanchor(&mut self, text: &str) {
        self.range = None;
        if self.quote.is_empty() {
            return;
        }
        let mut first = None;
        let mut count = 0;
        let mut contextual = None;
        let mut contextual_count = 0;
        for (start, _) in text.char_indices() {
            if !text[start..].starts_with(&self.quote) {
                continue;
            }
            let end = start + self.quote.len();
            let (Ok(start_byte), Ok(end_byte)) = (u32::try_from(start), u32::try_from(end)) else {
                continue;
            };
            let range = start_byte..end_byte;
            first = Some(range.clone());
            count += 1;
            if text[..start].ends_with(&self.before) && text[end..].starts_with(&self.after) {
                contextual = Some(range);
                contextual_count += 1;
            }
        }
        self.range = if count == 1 {
            first
        } else if contextual_count == 1 {
            contextual
        } else {
            None
        };
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

// Explicit stable FNV-1a, not DefaultHasher (whose algorithm is unspecified).
// The full path is also in the header: a collision fails safely, never overwrites.
fn identity(dir: &Path, file: &Path) -> io::Result<(PathBuf, String)> {
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()?.join(file)
    };
    let canonical = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut key = String::new();
    for &byte in canonical.as_os_str().as_encoded_bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        let _ = write!(key, "{byte:02x}");
    }
    Ok((
        dir.join("marginalia").join(format!("{hash:016x}.notes")),
        key,
    ))
}

fn decode(text: &str, key: &str) -> io::Result<Vec<Annotation>> {
    let mut lines = text.lines();
    if lines.next() != Some(format!("{HEADER}\t{key}").as_str()) {
        return Err(invalid(
            "annotation file has an unsupported version or different document path",
        ));
    }
    lines
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 6 {
                return Err(invalid("malformed annotation record"));
            }
            let range = match (fields[0], fields[1]) {
                ("-", "-") => None,
                (start, end) => {
                    let start: u32 = start
                        .parse()
                        .map_err(|_| invalid("invalid annotation start"))?;
                    let end: u32 = end.parse().map_err(|_| invalid("invalid annotation end"))?;
                    if start >= end {
                        return Err(invalid("empty or reversed annotation range"));
                    }
                    Some(start..end)
                }
            };
            let quote = unescape_field(fields[2]);
            if quote.is_empty()
                || range
                    .as_ref()
                    .is_some_and(|r| (r.end - r.start) as usize != quote.len())
            {
                return Err(invalid("annotation range does not match quote"));
            }
            Ok(Annotation {
                range,
                quote,
                note: unescape_field(fields[3]),
                before: unescape_field(fields[4]),
                after: unescape_field(fields[5]),
            })
        })
        .collect()
}

/// Load this document's sidecar and reanchor against its current display text.
/// A missing sidecar is empty; all other read and format errors propagate.
pub fn load_in(dir: &Path, file: &Path, text: &str) -> io::Result<Vec<Annotation>> {
    let (path, key) = identity(dir, file)?;
    let saved = match std::fs::read_to_string(path) {
        Ok(saved) => saved,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut annotations = decode(&saved, &key)?;
    for annotation in &mut annotations {
        annotation.reanchor(text);
    }
    Ok(annotations)
}

/// Atomically replace one document's sidecar without touching the source file.
/// Existing malformed data is never overwritten by this writer.
pub fn save_in(dir: &Path, file: &Path, annotations: &[Annotation]) -> io::Result<()> {
    let (path, key) = identity(dir, file)?;
    match std::fs::read_to_string(&path) {
        Ok(existing) => {
            decode(&existing, &key)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut output = format!("{HEADER}\t{key}\n");
    for annotation in annotations {
        let (start, end) = annotation.range.as_ref().map_or_else(
            || ("-".into(), "-".into()),
            |range| (range.start.to_string(), range.end.to_string()),
        );
        let _ = writeln!(
            output,
            "{start}\t{end}\t{}\t{}\t{}\t{}",
            escape_field(&annotation.quote),
            escape_field(&annotation.note),
            escape_field(&annotation.before),
            escape_field(&annotation.after)
        );
    }
    // Validate even callers constructing Annotation fields directly.
    decode(&output, &key)?;
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("missing sidecar directory"))?,
    )?;
    write_atomic(&path, &output)
}

/// Write a portable Markdown export inside the injected state directory.
pub fn export_in(dir: &Path, file: &Path, annotations: &[Annotation]) -> io::Result<PathBuf> {
    let (sidecar, _) = identity(dir, file)?;
    let path = sidecar.with_extension("md");
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("missing export directory"))?,
    )?;
    write_atomic(&path, &export_markdown(file, annotations))?;
    Ok(path)
}

/// Portable quote-and-note Markdown, including quotes no longer found in the file.
#[must_use]
pub fn export_markdown(file: &Path, annotations: &[Annotation]) -> String {
    let mut output = format!("# Notes\n\nDocument: {}\n", file.display());
    for (index, annotation) in annotations.iter().enumerate() {
        let _ = write!(output, "\n## {}\n\n", index + 1);
        if annotation.range.is_none() {
            output.push_str(
                "*Unresolved: this passage could not be located in the current document.*\n\n",
            );
        }
        for line in annotation.quote.split('\n') {
            let _ = writeln!(output, "> {line}");
        }
        if !annotation.note.is_empty() {
            let _ = write!(output, "\n{}\n", annotation.note);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_multiline_notes_round_trip_without_touching_source() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("文\t档\n\\.md");
        let source = "# Café\n\n你好 friends";
        std::fs::write(&file, source).unwrap();
        let text = "Café\n你好 friends";
        let annotation = Annotation::new(text, 6..12, "one\ntwo\t\\three\r 🦀".into()).unwrap();
        save_in(temp.path(), &file, std::slice::from_ref(&annotation)).unwrap();
        assert_eq!(load_in(temp.path(), &file, text).unwrap(), vec![annotation]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
    }

    #[test]
    fn reanchors_unique_quotes_and_retains_deleted_passages() {
        let mut a = Annotation::new("before café after", 7..12, "note".into()).unwrap();
        a.reanchor("inserted before café after");
        assert_eq!(a.range, Some(16..21));
        a.reanchor("nothing remains");
        assert_eq!(a.range, None);
        assert_eq!(a.quote, "café");
        a.reanchor("café returns");
        assert_eq!(a.range, Some(0..5));
    }

    #[test]
    fn repeated_passages_require_unique_context_even_at_old_offset() {
        let mut a = Annotation::new("cat", 0..3, String::new()).unwrap();
        a.reanchor("cat cat");
        assert_eq!(a.range, None);
        let mut a = Annotation::new("red cat blue", 4..7, String::new()).unwrap();
        a.reanchor("gray cat white; red cat blue");
        assert_eq!(a.range, Some(20..23));
        a.reanchor("red cat blue; red cat blue");
        assert_eq!(a.range, None);
    }

    #[test]
    fn overlapping_matches_are_ambiguous() {
        let mut a = Annotation::new("aa", 0..2, String::new()).unwrap();
        a.reanchor("aaa");
        assert_eq!(a.range, None);
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        for range in [0..0, Range { start: 4, end: 2 }, 0..99, 1..2] {
            assert!(Annotation::new("école", range, String::new()).is_none());
        }
    }

    #[test]
    fn documents_are_independent_and_empty_saves_clear_only_one() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.md");
        let second = temp.path().join("second.md");
        let a = Annotation::new("word", 0..4, "note".into()).unwrap();
        assert!(load_in(temp.path(), &first, "word").unwrap().is_empty());
        save_in(temp.path(), &first, std::slice::from_ref(&a)).unwrap();
        save_in(temp.path(), &second, std::slice::from_ref(&a)).unwrap();
        save_in(temp.path(), &first, &[]).unwrap();
        assert!(load_in(temp.path(), &first, "word").unwrap().is_empty());
        assert_eq!(load_in(temp.path(), &second, "word").unwrap(), vec![a]);
    }

    #[test]
    fn corruption_and_io_errors_are_reported_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.md");
        save_in(temp.path(), &file, &[]).unwrap();
        let (path, _) = identity(temp.path(), &file).unwrap();
        std::fs::write(&path, "broken").unwrap();
        assert!(load_in(temp.path(), &file, "").is_err());
        assert!(save_in(temp.path(), &file, &[]).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
        let blocked = temp.path().join("blocked");
        std::fs::write(&blocked, "file").unwrap();
        assert!(save_in(&blocked, &file, &[]).is_err());
    }

    #[test]
    fn export_writes_under_state_and_reports_write_failures() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("source.md");
        std::fs::write(&file, "word").unwrap();
        let a = Annotation::new("word", 0..4, "a note".into()).unwrap();
        let state = temp.path().join("state");
        let export = export_in(&state, &file, std::slice::from_ref(&a)).unwrap();
        assert!(export.starts_with(&state));
        assert_eq!(
            std::fs::read_to_string(&export).unwrap(),
            export_markdown(&file, &[a])
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "word");
        std::fs::remove_file(&export).unwrap();
        std::fs::create_dir(&export).unwrap();
        assert!(export_in(&state, &file, &[]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_spellings_share_annotations() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("source.md");
        let alias = temp.path().join("alias.md");
        std::fs::write(&file, "word").unwrap();
        std::os::unix::fs::symlink(&file, &alias).unwrap();
        let a = Annotation::new("word", 0..4, "note".into()).unwrap();
        save_in(temp.path(), &file, std::slice::from_ref(&a)).unwrap();
        assert_eq!(load_in(temp.path(), &alias, "word").unwrap(), vec![a]);
    }

    #[test]
    fn unresolved_annotations_round_trip_and_export_quote_and_note() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.md");
        let mut a = Annotation::new("first\nsecond", 0..12, "my\nnote".into()).unwrap();
        a.reanchor("");
        save_in(temp.path(), &file, std::slice::from_ref(&a)).unwrap();
        assert_eq!(load_in(temp.path(), &file, "").unwrap(), vec![a.clone()]);
        let exported = export_markdown(&file, &[a]);
        assert!(exported.contains("Unresolved"));
        assert!(exported.contains("> first\n> second\n\nmy\nnote\n"));
    }
}
