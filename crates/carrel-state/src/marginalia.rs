//! Reader annotations anchored in display-text bytes, stored apart from documents.
//!
//! Callers inject the state directory. Missing files mean no annotations; malformed
//! records and I/O failures are errors so valuable notes are never silently lost.

use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::state::{escape_field, locked, unescape_field, write_atomic, write_private};

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

/// `read_to_string`, refusing anything that is not a regular file rather
/// than blocking on it: a FIFO where a sidecar should be hung the open.
fn read_sidecar(path: &Path) -> io::Result<String> {
    String::from_utf8(crate::state::read_regular(path)?).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })
}

/// Load this document's sidecar and reanchor against its current display text.
/// A missing sidecar is empty; all other read and format errors propagate.
pub fn load_in(dir: &Path, file: &Path, text: &str) -> io::Result<Vec<Annotation>> {
    let (path, key) = identity(dir, file)?;
    let mut annotations = stored(&path, &key)?;
    for annotation in &mut annotations {
        annotation.reanchor(text);
    }
    Ok(annotations)
}

/// Atomically replace one document's sidecar without touching the source file.
/// Existing malformed data is never overwritten by this writer.
///
/// The list replaces the sidecar whole, so this is for a caller that knows
/// no other reader has the document open; a reader saves with [`sync_in`].
pub fn save_in(dir: &Path, file: &Path, annotations: &[Annotation]) -> io::Result<()> {
    let (path, key) = identity(dir, file)?;
    std::fs::create_dir_all(dir)?;
    locked(dir, || {
        stored(&path, &key)?;
        write_sidecar(&path, &key, annotations)
    })
}

/// Save what this session did to the notes, not what it holds: `mine` is
/// merged with the sidecar as it is on disk NOW, against `base` — the list
/// as this session last read or wrote it — and the result is written and
/// returned, for the caller to show and to use as its next `base`.
///
/// Two readers on one document each held the whole list and each wrote the
/// whole list, so the second to save erased the first one's notes. Here a
/// note another reader added since `base` is kept, one it deleted is not
/// brought back, and this session's own additions, edits and deletions
/// apply (`merge` has the rule and what happens when both touched one
/// note). The read, the merge and the write share one lock.
///
/// A note that came from disk is placed in `text`, the open document's
/// display text, as [`load_in`] would place it. Existing malformed data is
/// never overwritten, as with [`save_in`].
pub fn sync_in(
    dir: &Path,
    file: &Path,
    text: &str,
    base: &[Annotation],
    mine: &[Annotation],
) -> io::Result<Vec<Annotation>> {
    let (path, key) = identity(dir, file)?;
    std::fs::create_dir_all(dir)?;
    locked(dir, || {
        let disk = stored(&path, &key)?;
        let merged = merge(text, base, mine, &disk);
        write_sidecar(&path, &key, &merged)?;
        Ok(merged)
    })
}

/// What a note is, apart from what it says: the passage it is on. The
/// range is not part of it — that is derived, and moves with every edit of
/// the document.
fn passage(a: &Annotation) -> (&str, &str, &str) {
    (&a.quote, &a.before, &a.after)
}

/// Each note's index among the notes on the same passage, so two on one
/// passage (a repeated line with identical surroundings) stay two.
fn nth(list: &[Annotation]) -> Vec<usize> {
    (0..list.len())
        .map(|i| {
            list[..i]
                .iter()
                .filter(|a| passage(a) == passage(&list[i]))
                .count()
        })
        .collect()
}

fn find<'a>(
    list: &'a [Annotation],
    nths: &[usize],
    a: &Annotation,
    n: usize,
) -> Option<&'a Annotation> {
    list.iter()
        .zip(nths)
        .find(|&(b, &k)| k == n && passage(b) == passage(a))
        .map(|(b, _)| b)
}

/// Two texts for one note, when both readers wrote one: neither is dropped.
fn both(mine: &str, theirs: &str) -> String {
    if theirs.is_empty() || mine.contains(theirs) {
        mine.into()
    } else if mine.is_empty() {
        theirs.into()
    } else {
        format!("{mine}\n\n{theirs}")
    }
}

/// The three-way merge behind [`sync_in`], note by note:
///
/// - untouched here: whatever the disk says, including "deleted";
/// - added, edited or deleted here and untouched there: what was done here;
/// - touched by both: nothing anyone wrote is lost — an edit beats a
///   delete on either side, and two different texts are both kept, this
///   session's first.
///
/// This session's notes keep their order (the pane's rows are indexes into
/// it); another reader's additions follow.
fn merge(
    text: &str,
    base: &[Annotation],
    mine: &[Annotation],
    disk: &[Annotation],
) -> Vec<Annotation> {
    let (base_n, mine_n, disk_n) = (nth(base), nth(mine), nth(disk));
    let mut out = Vec::with_capacity(mine.len());
    for (m, &n) in mine.iter().zip(&mine_n) {
        let b = find(base, &base_n, m, n);
        let d = find(disk, &disk_n, m, n);
        let note = match (b, d) {
            // Deleted there and not edited here.
            (Some(b), None) if b.note == m.note => continue,
            (Some(_) | None, None) => m.note.clone(),
            (Some(b), Some(d)) if b.note == m.note => d.note.clone(),
            (Some(b), Some(d)) if b.note == d.note => m.note.clone(),
            (_, Some(d)) => both(&m.note, &d.note),
        };
        out.push(Annotation { note, ..m.clone() });
    }
    for (d, &n) in disk.iter().zip(&disk_n) {
        if find(mine, &mine_n, d, n).is_some() {
            continue;
        }
        // Deleted here — unless it was edited there since.
        if find(base, &base_n, d, n).is_some_and(|b| b.note == d.note) {
            continue;
        }
        let mut d = d.clone();
        d.reanchor(text);
        out.push(d);
    }
    out
}

/// The sidecar's notes as they are stored; a missing sidecar is empty.
/// Every read of one comes through here, and so through [`read_sidecar`].
fn stored(path: &Path, key: &str) -> io::Result<Vec<Annotation>> {
    match read_sidecar(path) {
        Ok(existing) => decode(&existing, key),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn write_sidecar(path: &Path, key: &str, annotations: &[Annotation]) -> io::Result<()> {
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
    decode(&output, key)?;
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("missing sidecar directory"))?,
    )?;
    write_private(path, &output)
}

/// Where [`export_in`] writes: `exports/<stem>-notes-<8 hex>.md` under the
/// state directory.
///
/// Never beside the document. A reader must not leave files in someone's
/// repository unasked — an untracked `.md` there shows up in `git status`,
/// in carrel's own home list, and in whatever the agent next to it reads —
/// and the document's directory may be read-only, or a piped document may
/// have none. The state directory is carrel's own and always the same
/// place, so the export goes there and the caller reports the exact path.
///
/// The name leads with the document's stem so a person browsing the
/// directory can find it; the hash is the sidecar's own, cut to 8 hex, so
/// two `README.md`s in different directories never overwrite each other's
/// export. Control characters in the stem become `_` and it is capped, so a
/// hostile file name cannot smuggle an escape into the note that prints it.
fn export_path(dir: &Path, file: &Path) -> io::Result<PathBuf> {
    let (sidecar, _) = identity(dir, file)?;
    let hash = sidecar
        .file_stem()
        .and_then(|s| s.to_str())
        .map_or("", |s| &s[..s.len().min(8)])
        .to_string();
    let stem: String = file
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(48)
        .collect();
    let stem = if stem.trim().is_empty() {
        "document".into()
    } else {
        stem
    };
    Ok(dir.join("exports").join(format!("{stem}-notes-{hash}.md")))
}

/// Write a portable Markdown export inside the injected state directory.
pub fn export_in(dir: &Path, file: &Path, annotations: &[Annotation]) -> io::Result<PathBuf> {
    let path = export_path(dir, file)?;
    std::fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("missing export directory"))?,
    )?;
    write_atomic(&path, export_markdown(file, annotations))?;
    Ok(path)
}

/// Portable quote-and-note Markdown, including quotes no longer found in the file.
#[must_use]
pub fn export_markdown(file: &Path, annotations: &[Annotation]) -> String {
    // One line, whatever the file is called: this is pasted (`copy all`),
    // and a newline in a file name would start a second command line.
    let shown = crate::clip::line(&file.display().to_string());
    let mut output = format!("# Notes\n\nDocument: {shown}\n");
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
        assert_eq!(
            load_in(temp.path(), &first, "word").unwrap(),
            [] as [crate::marginalia::Annotation; 0]
        );
        save_in(temp.path(), &first, std::slice::from_ref(&a)).unwrap();
        save_in(temp.path(), &second, std::slice::from_ref(&a)).unwrap();
        save_in(temp.path(), &first, &[]).unwrap();
        assert_eq!(
            load_in(temp.path(), &first, "word").unwrap(),
            [] as [crate::marginalia::Annotation; 0]
        );
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
        assert_eq!(export.parent(), Some(state.join("exports").as_path()));
        let name = export.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("source-notes-"), "{name}");
        assert_eq!(export.extension().and_then(|e| e.to_str()), Some("md"));
        assert_eq!(
            std::fs::read_to_string(&export).unwrap(),
            export_markdown(&file, &[a])
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "word");
        std::fs::remove_file(&export).unwrap();
        std::fs::create_dir(&export).unwrap();
        assert!(export_in(&state, &file, &[]).is_err());
    }

    /// An export is named after its document so a person can find it, and
    /// still keyed to the full path so two `README.md`s never overwrite
    /// each other's. A hostile file name cannot put a control character or
    /// an unbounded run into the name.
    #[test]
    fn exports_are_named_for_their_document_and_never_collide() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let a = Annotation::new("word", 0..4, "note".into()).unwrap();
        let mut paths = Vec::new();
        for dir in ["one", "two"] {
            std::fs::create_dir(temp.path().join(dir)).unwrap();
            let file = temp.path().join(dir).join("README.md");
            std::fs::write(&file, "word").unwrap();
            paths.push(export_in(&state, &file, std::slice::from_ref(&a)).unwrap());
        }
        assert_ne!(paths[0], paths[1]);
        for p in &paths {
            let name = p.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with("README-notes-"), "{name}");
        }
        let hostile = temp
            .path()
            .join(format!("a\tb\n\u{1b}[31m{}.md", "x".repeat(200)));
        std::fs::write(&hostile, "word").unwrap();
        let p = export_in(&state, &hostile, std::slice::from_ref(&a)).unwrap();
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(!name.chars().any(char::is_control), "{name:?}");
        assert!(name.chars().count() < 100, "{name:?}");
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

    fn on(quote: &str, note: &str) -> Annotation {
        let text = "alpha beta gamma delta";
        let start = u32::try_from(text.find(quote).unwrap()).unwrap();
        let end = start + u32::try_from(quote.len()).unwrap();
        Annotation::new(text, start..end, note.into()).unwrap()
    }

    fn said(list: &[Annotation]) -> Vec<String> {
        list.iter()
            .map(|a| format!("{}={}", a.quote, a.note))
            .collect()
    }

    /// Every case the merge has, one passage each: what this session did
    /// against what another did since the baseline.
    #[test]
    fn sync_merges_each_note_three_ways() {
        let text = "alpha beta gamma delta";
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.md");
        let base = [on("alpha", "a"), on("beta", "b"), on("gamma", "g")];
        save_in(temp.path(), &file, &base).unwrap();
        // Another session: edits alpha, deletes beta, adds delta.
        let theirs = [on("alpha", "a2"), on("gamma", "g"), on("delta", "d")];
        save_in(temp.path(), &file, &theirs).unwrap();
        // This one: edits gamma, still holds beta, adds a highlight on "be".
        let mine = [
            on("alpha", "a"),
            on("beta", "b"),
            on("gamma", "g2"),
            on("be", ""),
        ];
        let merged = sync_in(temp.path(), &file, text, &base, &mine).unwrap();
        assert_eq!(said(&merged), ["alpha=a2", "gamma=g2", "be=", "delta=d"]);
        assert!(merged.iter().all(|a| a.range.is_some()));
        assert_eq!(load_in(temp.path(), &file, text).unwrap(), merged);
        // Nothing changed anywhere: a save is the file as it is.
        let again = sync_in(temp.path(), &file, text, &merged, &merged).unwrap();
        assert_eq!(again, merged);
    }

    #[test]
    fn sync_never_drops_text_either_side_wrote() {
        let text = "alpha beta gamma delta";
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.md");
        let base = [on("alpha", "a"), on("beta", "b"), on("gamma", "g")];
        // Deleted here, edited there: the edit stays.
        // Edited here, deleted there: the edit stays.
        // Edited on both sides, differently: both texts, this one's first.
        save_in(
            temp.path(),
            &file,
            &[on("alpha", "a!"), on("gamma", "theirs")],
        )
        .unwrap();
        let mine = [on("beta", "b!"), on("gamma", "mine")];
        let merged = sync_in(temp.path(), &file, text, &base, &mine).unwrap();
        assert_eq!(
            said(&merged),
            ["beta=b!", "gamma=mine\n\ntheirs", "alpha=a!"]
        );
        // Both adding a bare highlight on one passage is one highlight, and
        // a note on a passage the other only highlighted is that note.
        let temp = tempfile::tempdir().unwrap();
        save_in(temp.path(), &file, &[on("alpha", ""), on("beta", "")]).unwrap();
        let mine = [on("alpha", ""), on("beta", "note")];
        let merged = sync_in(temp.path(), &file, text, &[], &mine).unwrap();
        assert_eq!(said(&merged), ["alpha=", "beta=note"]);
    }

    #[test]
    fn sync_refuses_a_sidecar_it_cannot_read_and_leaves_it() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.md");
        save_in(temp.path(), &file, &[]).unwrap();
        let (path, _) = identity(temp.path(), &file).unwrap();
        std::fs::write(&path, "broken").unwrap();
        assert!(sync_in(temp.path(), &file, "", &[], &[]).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }
}
