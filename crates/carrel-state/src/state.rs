//! Reading positions — the XDG *state* dir, not config.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. And no reaching the
//! real directory from library code: everything flows through
//! `App::state_dir`, which is `None` in every constructor, for the same
//! reason `config.rs` works that way (a test once stomped the developer's
//! real config; see the comment there).
//!
//! Format: one entry per line,
//! `anchor<TAB>saved_at<TAB>permille<TAB>words<TAB>path`, the path
//! **escaped** by `escape_path` (`escape_field`, plus `\xNN` for a byte
//! that is not UTF-8). **The old three-field form still
//! parses** — an entry written before 2026-08-21 simply has no progress to
//! show, rather than being dropped — and malformed lines are ignored, so a
//! newer version's file cannot break an older binary. **Nor is it lost:** a
//! line, or a bookmark, this version cannot read is written back unchanged
//! when it rewrites the file. Releases up to 2026.10.21 did not do this, so
//! going back to one of them discards what a newer carrel saved.
//!
//! The anchor, and every bookmark, is a [`Place`]: a SOURCE byte, written
//! `s<src>` with a signed correction after it (`s1024+16`). Releases up to
//! 2026.10.21 wrote a bare display byte, which still reads, as
//! [`Place::Legacy`]. See [`Place`] for why the display byte had to go.
//!
//! `permille` and `words` exist so the home screen can say "64%, 18 min
//! left" without opening the file. Opening every remembered document to
//! paint a list of them is not affordable; both numbers are already
//! computed by the status bar at the moment a position is saved.
//!
//! # The on-disk helpers live here
//!
//! `write_atomic` and `escape_field` are `pub(crate)` and used by
//! `config.rs` and `scan.rs` as well. They belong to one module because two
//! spellings of "escape a path into a line" is exactly how the bookmarks
//! file came to be corruptible; this is the module whose whole subject is
//! records carrel writes to disk, so it is the one that owns them.

use std::path::{Path, PathBuf};

/// Replace `path`'s contents with `text` without ever truncating it in place.
///
/// `fs::write` opens with `O_TRUNC` and then writes, so the file is empty for
/// as long as the write takes. Every file this project keeps is a
/// read-modify-write of *everything* it holds — up to `CAP` reading
/// positions — so a crash, a SIGKILL or a full disk inside that window does
/// not cost the last line, it costs all of them, and a second carrel reading
/// at that instant sees a truncated file. Writing a sibling temp file and
/// `rename`-ing it onto the target closes the window: `rename(2)` within one
/// directory is atomic, so an observer sees either the whole old file or the
/// whole new one. The temp file is a sibling rather than in `/tmp` because
/// `rename` is only atomic — only *possible*, in fact — within a filesystem.
///
/// `sync_all` before the rename is what makes the crash case hold and not
/// just the concurrent-reader one: without it the rename can reach the disk
/// before the data does, and the survivor is a correctly-named empty file.
///
/// **This does not make concurrent writers safe, and is not meant to.** Two
/// carrels in two terminals still race — each reads the whole file, each
/// rewrites the whole file, and the later `rename` wins entire, so the loser's
/// session is discarded. That is last-writer-wins, unchanged from before;
/// what changes is that the loser's outcome is a coherent older file rather
/// than a shredded one. Doing better needs a lock file, which needs a policy
/// for the stale lock a killed reader leaves behind, and that is a worse
/// trade for state this size.
///
/// **A symlink is written through, not replaced.** `rename` onto a link
/// replaces the LINK, so a stow-managed `~/.config/carrel/config` pointing
/// into a dotfiles repo silently became a regular file the first time a
/// theme was cycled, and the repo's copy stopped changing. The target is
/// resolved first and the temp file goes beside it, so the rename stays
/// within the target's filesystem and the link keeps pointing where it did.
pub(crate) fn write_atomic(path: &Path, text: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write as _;
    let resolved;
    let path = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            // A dangling link has nothing to canonicalize; write where it
            // points, which is what the link says the file is.
            resolved = std::fs::canonicalize(path).or_else(|_| {
                let parent = path.parent().unwrap_or_else(|| Path::new("."));
                std::fs::read_link(path).map(|l| parent.join(l))
            })?;
            resolved.as_path()
        }
        _ => path,
    };
    let text = text.as_ref();
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // The pid keeps two carrels from writing the same temp file and renaming
    // each other's half-written bytes into place.
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let write = |tmp: &Path| -> std::io::Result<()> {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(text)?;
        f.sync_all()
    };
    if let Err(e) = write(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Escape a path so it survives as one field of one TAB-separated line.
///
/// Unix forbids only `/` and NUL in a filename, so a TAB or a NEWLINE in a
/// path is legal and does turn up. Unescaped, the tab splits the record
/// *inside* the path so no entry ever matches again, and the newline splits
/// one record into two — which also means a crafted filename can inject a
/// forged entry for another document.
///
/// The escape is the C one, `\\` `\t` `\n` (and `\r`, which is not a
/// separator here but would still be a nasty thing to write into a line
/// verbatim). It is chosen because it is its own inverse under
/// [`unescape_field`] and because **an already-written file reads back
/// unchanged**: a path with none of these characters escapes to itself, and
/// unescaping is identity on any string without a backslash. The one thing
/// it cannot recover is a path containing a literal backslash written by a
/// version older than this one — a `\t` on disk from then is read as a tab.
/// That is a strictly smaller set than the paths broken before the fix.
pub(crate) fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// The inverse of `escape_field`. An unknown escape keeps both characters,
/// so a path this version never wrote is returned as it was found rather than
/// silently mangled.
pub(crate) fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            // An escaped backslash, and a trailing one with nothing after it:
            // both are one literal backslash.
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// [`escape_field`] for a path, losslessly.
///
/// A path is bytes, not text. `display()` turned every byte that is not
/// UTF-8 into U+FFFD, so `\xff.md` and `\xfe.md` shared one position, one
/// bookmark list and one set of folds, and the home screen offered a
/// `\u{fffd}.md` that does not exist. Each such byte is written `\xNN`
/// instead. A path that IS UTF-8 escapes exactly as before, so every line
/// already on disk still matches; and since `escape_field` writes a
/// backslash as `\\`, no line this version or the last wrote has `\x` in
/// it to be misread.
fn escape_path(p: &Path) -> String {
    #[cfg(unix)]
    {
        use std::fmt::Write as _;
        use std::os::unix::ffi::OsStrExt as _;
        let mut out = String::new();
        for chunk in p.as_os_str().as_bytes().utf8_chunks() {
            out.push_str(&escape_field(chunk.valid()));
            for b in chunk.invalid() {
                let _ = write!(out, "\\x{b:02x}");
            }
        }
        out
    }
    #[cfg(not(unix))]
    {
        escape_field(&p.to_string_lossy())
    }
}

/// The inverse of [`escape_path`], with [`unescape_field`]'s rule that an
/// escape it does not know keeps both characters.
fn unescape_path(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let hex = b
            .get(i + 2..i + 4)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (b.get(i + 1), hex) {
            (Some(b't'), _) => out.push(b'\t'),
            (Some(b'n'), _) => out.push(b'\n'),
            (Some(b'r'), _) => out.push(b'\r'),
            (Some(b'\\'), _) => out.push(b'\\'),
            (Some(b'x'), Some(byte)) => {
                out.push(byte);
                i += 4;
                continue;
            }
            // A trailing backslash, or an escape this version never wrote:
            // the backslash stays, and what follows is read as itself.
            _ => {
                out.push(b'\\');
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        PathBuf::from(std::ffi::OsString::from_vec(out))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&out).into_owned())
    }
}

/// A state file's bytes; a file that is not there yet is empty.
///
/// **Any other failure is an error, and a save stops at it.** Every save
/// here rewrites the whole file from what it read, so a file it could not
/// read — no permission, an I/O error — would be replaced by the one entry
/// being saved. `marginalia::save_in` refuses for the same reason. The
/// callers drop the error, so the refusal is quiet.
fn read_state(path: &Path) -> std::io::Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// The non-blank lines of a state file, as BYTES.
///
/// Bytes because `read_to_string` fails on a single byte that is not UTF-8
/// — a torn write from a release that still truncated in place, a disk
/// error — and the save that followed read "nothing" and wrote back only
/// its own entry: every other document's position gone. Reading line by
/// line, rather than refusing the whole file, means one bad line costs only
/// itself: it is unreadable, so it is kept verbatim like a line from a
/// newer carrel, and every line around it goes on working.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|&b| b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .filter(|l| !l.trim_ascii().is_empty())
}

/// Keep the most recent N files. Positions are a convenience, not a
/// database; the cap stops the file growing forever.
const CAP: usize = 500;

/// The same cap for bookmarks, and for the same reason.
const MARK_CAP: usize = 500;

/// Where a saved place is in a document, in coordinates that outlive the
/// version of carrel that saved it.
///
/// The reader's position is a display byte (`DocByte`), an offset into
/// `Document::text`. That text is DERIVED: it is whatever carrel-core's
/// flattening rules make of the source, and those rules change between
/// releases (frontmatter padding moved from paint to parse, tabs are
/// expanded at parse, inline math enters the text rendered). A display
/// byte saved by one carrel therefore names a different place under the
/// next — and with a terminal and a GUI installed from different channels,
/// two cores reading one state file is the normal case, not an upgrade's.
/// The source bytes are the author's and change only when the file does —
/// with one exception: a `.diff` or `.patch` (or a pipe) read as a diff is
/// shown as markdown carrel generates from it, and its "source" is that
/// markdown, which a newer carrel may generate differently (and which
/// `--diff`/`--no-diff` change outright). A place in one keeps the old
/// guarantee only: right under the core that saved it.
///
/// A source byte alone is not enough, though. Provenance is recorded per
/// RUN, and a run whose text was rewritten — a paragraph with smart quotes
/// in it, an autolink — maps every display byte inside it to the run's
/// first source byte, so a reading position saved halfway down such a
/// paragraph would come back at its top, rows earlier. So a place also
/// keeps `delta`: how far the display byte stood from where its source
/// byte maps back to. Under the core that saved it, and with the file as it
/// was, `to_doc(src) + delta` is the saved display byte EXACTLY; under another, `to_doc(src)` finds
/// the right run wherever the new rules put it, and the correction, which
/// is never larger than a run, keeps the place inside it as nearly as the
/// new text allows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A display byte, as releases up to 2026.10.21 wrote one: correct
    /// only under the core that flattened it, and read as such.
    Legacy(u32),
    /// A source byte and a correction in display bytes — and, when it was
    /// saved by [`Place::kept`], the block it was in, by content.
    Src {
        src: u32,
        delta: i32,
        block: Option<Kept>,
    },
}

/// The block a place was in, by what it said: the second answer, for a
/// file edited while carrel was closed.
///
/// A source byte names the same place only while the file is unchanged. A
/// paragraph inserted above moves every byte below it, and a reopened
/// bookmark landed on whatever paragraph had moved into its bytes. A live
/// reload carried bookmarks by content all along (`App::carry_marks`); a
/// cold reopen had no content to match, so now the place keeps it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// `block_hash` of the block's display text.
    pub hash: u64,
    /// Which of the blocks with that text it was: 0 for the first.
    pub nth: u32,
    /// The display byte, from the block's start. Negative for the blank row
    /// above it, which a reader's view names by the separator before it.
    pub off: i32,
    /// `block_hash` of the block before it ([`FIRST`] for the first
    /// block): which copy of a repeated text it was, when the count alone
    /// cannot say. `None` in a place saved before the field existed.
    pub prev: Option<u64>,
}

/// The `prev` of a document's first block, which has none.
pub const FIRST: u64 = 0;

/// Which of a document's blocks kept place `k` names, given each block's
/// hash (`hash_of(i)`, any hash, so long as `k` was made with the same).
///
/// A text that appears once is that block. One that repeats — a CHANGELOG's
/// `### Fixed` under every release — is the copy with the same block before
/// it, so a copy inserted above (a new release at the top) does not take
/// the place over; of several such, the one whose count is nearest the one
/// saved. Where no copy keeps its neighbour, the `nth` copy, as far as it
/// still exists: a neighbour was edited. Where it no longer does, `None` —
/// the block itself is gone, and the caller's own fallback (the source
/// place) is nearer than any other copy, which may be fifty paragraphs
/// away. A place saved without `prev` keeps the old rule: by count, and the
/// last copy when there are fewer.
pub fn pick(count: usize, hash_of: impl Fn(usize) -> u64, k: Kept) -> Option<usize> {
    let nth = k.nth as usize;
    let mut rank = 0usize;
    let mut by_count = None;
    let mut last = None;
    let mut ctx: Option<(usize, usize)> = None; // (distance in count, block)
    let mut before = FIRST;
    for i in 0..count {
        let h = hash_of(i);
        if h == k.hash {
            if rank == nth {
                by_count = Some(i);
            }
            last = Some(i);
            if k.prev == Some(before) {
                let d = rank.abs_diff(nth);
                if ctx.is_none_or(|(best, _)| d < best) {
                    ctx = Some((d, i));
                }
            }
            rank += 1;
        }
        before = h;
    }
    match k.prev {
        Some(_) => ctx.map(|(_, i)| i).or(by_count),
        None => by_count.or(last),
    }
}

/// A block's text, hashed the same way by every build: FNV-1a over its
/// display bytes. Written out by hand because the standard library's
/// hasher promises nothing across Rust releases, and this one is saved.
fn block_hash(doc: &carrel_core::Document, b: carrel_core::BlockIdx) -> u64 {
    let r = &doc.node_for_block(b).doc;
    doc.text.as_bytes()[r.start as usize..r.end as usize]
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, &x| {
            (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

impl Place {
    /// The place of display byte `d` in `doc`.
    #[must_use]
    pub fn of(doc: &carrel_core::Document, d: carrel_core::DocByte) -> Self {
        let src = doc.to_src(d);
        let back = doc.to_doc(src);
        let delta = i64::from(d.0) - i64::from(back.0);
        Self::Src {
            src: src.0,
            delta: i32::try_from(delta).unwrap_or(0),
            block: None,
        }
    }

    /// [`Self::of`], keeping the block `d` is in by its content, so the
    /// place can be found again in the file after an edit. What a reading
    /// position and a bookmark are saved as. Hashes every block up to
    /// `d`'s, so it is for saving, not for every byte of a document.
    #[must_use]
    pub fn kept(doc: &carrel_core::Document, d: carrel_core::DocByte) -> Self {
        let Self::Src { src, delta, .. } = Self::of(doc, d) else {
            unreachable!("`of` spells a source place")
        };
        if doc.block_count() == 0 {
            return Self::Src {
                src,
                delta,
                block: None,
            };
        }
        let b = doc.block_at_doc(d);
        let hash = block_hash(doc, b);
        let nth = (0..b.get())
            .filter(|&i| block_hash(doc, carrel_core::BlockIdx(i as u32)) == hash)
            .count();
        let off = i64::from(d.0) - i64::from(doc.node_for_block(b).doc.start);
        let prev = match b.get() {
            0 => FIRST,
            i => block_hash(doc, carrel_core::BlockIdx(i as u32 - 1)),
        };
        Self::Src {
            src,
            delta,
            block: Some(Kept {
                hash,
                nth: u32::try_from(nth).unwrap_or(u32::MAX),
                off: i32::try_from(off).unwrap_or(0),
                prev: Some(prev),
            }),
        }
    }

    /// The display byte this place names in `doc`: inside the text, and on
    /// a character boundary, whatever the file or the core did since.
    ///
    /// A place that kept its block goes where that block's text is now —
    /// the same one of several, as a live reload decides ([`pick`]) — so an
    /// edit elsewhere in the file does not move it. A block that was itself
    /// edited or removed has no text to find, and the source place stands.
    #[must_use]
    pub fn resolve(self, doc: &carrel_core::Document) -> carrel_core::DocByte {
        let at = match self {
            Self::Legacy(d) => i64::from(d),
            Self::Src { src, delta, block } => {
                let at = i64::from(doc.to_doc(carrel_core::SrcByte(src)).0) + i64::from(delta);
                block.and_then(|k| found(doc, k)).unwrap_or(at)
            }
        };
        let last = doc.text.len().saturating_sub(1);
        let mut at = usize::try_from(at.max(0)).unwrap_or(usize::MAX).min(last);
        while at > 0 && !doc.text.is_char_boundary(at) {
            at -= 1;
        }
        carrel_core::DocByte(at as u32)
    }

    /// Whether this place's block is in `doc` — false only for a place
    /// that kept its block ([`Self::kept`]) whose text `doc` no longer
    /// has, where [`Self::resolve`] falls back to the source place.
    #[must_use]
    pub fn finds_its_block(self, doc: &carrel_core::Document) -> bool {
        match self {
            Self::Src { block: Some(k), .. } => found(doc, k).is_some(),
            _ => true,
        }
    }

    fn encode(self) -> String {
        match self {
            Self::Legacy(d) => d.to_string(),
            Self::Src {
                src,
                delta,
                block: None,
            } => format!("s{src}{delta:+}"),
            Self::Src {
                src,
                delta,
                block: Some(k),
            } => {
                let mut f = format!("s{src}{delta:+}@{:016x}.{}{:+}", k.hash, k.nth, k.off);
                if let Some(p) = k.prev {
                    use std::fmt::Write as _;
                    let _ = write!(f, "^{p:016x}");
                }
                f
            }
        }
    }

    /// `None` for anything this version did not write — a field from a
    /// newer carrel is skipped, never misread as a place.
    fn decode(field: &str) -> Option<Self> {
        let Some(rest) = field.strip_prefix('s') else {
            return field.parse().ok().map(Self::Legacy);
        };
        let (rest, kept) = match rest.split_once('@') {
            Some((r, k)) => (r, Some(k)),
            None => (rest, None),
        };
        let at = rest.find(['+', '-'])?;
        let block = match kept {
            None => None,
            Some(k) => {
                let (hash, k) = k.split_once('.')?;
                let (k, prev) = match k.split_once('^') {
                    Some((k, p)) if p.len() == 16 => (k, Some(u64::from_str_radix(p, 16).ok()?)),
                    Some(_) => return None,
                    None => (k, None),
                };
                let at = k.find(['+', '-'])?;
                if hash.len() != 16 {
                    return None;
                }
                Some(Kept {
                    hash: u64::from_str_radix(hash, 16).ok()?,
                    nth: k[..at].parse().ok()?,
                    off: k[at..].parse().ok()?,
                    prev,
                })
            }
        };
        Some(Self::Src {
            src: rest[..at].parse().ok()?,
            delta: rest[at..].parse().ok()?,
            block,
        })
    }
}

/// Where kept block `k` is in `doc`: the same one of the blocks with its
/// text, by count, at the same offset. `None` when no block has its text
/// any more. For an unchanged file this IS the saved byte, so it is asked
/// first; asking instead "is the source place already in a block with this
/// text?" was fooled by a stale place on the separator before the right
/// block's start, which `block_at_doc` gives to that block.
fn found(doc: &carrel_core::Document, k: Kept) -> Option<i64> {
    let b = pick(
        doc.block_count(),
        |i| block_hash(doc, carrel_core::BlockIdx(i as u32)),
        k,
    )?;
    let node = doc.node_for_block(carrel_core::BlockIdx(b as u32));
    Some((i64::from(node.doc.start) + i64::from(k.off)).min(i64::from(node.doc.end)))
}

/// `$XDG_STATE_HOME/carrel`, else `~/.local/state/carrel`.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x).join("carrel"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".local").join("state").join("carrel"))
}

/// Has this reader ever remembered a reading position?
///
/// The absence of the `positions` file, which is written the first time a
/// document is read far enough to be worth resuming — so the first-run
/// invitation stays until you have actually read something, and then never
/// comes back. A session that opens a document, reads nothing and quits
/// writes nothing and is offered the line again, which is the right answer:
/// nothing was learned.
#[must_use]
pub fn is_first_run_in(dir: &Path) -> bool {
    !dir.join("positions").exists()
}

/// One remembered document.
#[derive(Debug, Clone)]
pub struct Entry {
    pub place: Place,
    pub saved_at: u64,
    /// How far in, in permille. `None` for an entry written before the
    /// field existed.
    pub permille: Option<u16>,
    /// The document's word count when it was last read, for the estimate.
    pub words: Option<u32>,
    /// The document, byte for byte — not a `String`, because a path need
    /// not be UTF-8 and a lossy one names a file that does not exist.
    pub path: PathBuf,
}

/// Every entry this version can read, in file order. A line that is not
/// UTF-8 is simply not one of them.
fn parse(bytes: &[u8]) -> impl Iterator<Item = Entry> + '_ {
    lines(bytes).filter_map(|l| parse_line(std::str::from_utf8(l).ok()?))
}

fn parse_line(line: &str) -> Option<Entry> {
    // Split on EVERY tab and require the exact count. The path is escaped,
    // so it can never hold a raw tab, and a line with more fields than this
    // version writes is a newer carrel's. `splitn(5)` read
    // `s10+0 ⇥ … ⇥ newfield=1 ⇥ /docs/plan.md` as the path
    // `newfield=1⇥/docs/plan.md`, and the next save wrote it back with the
    // tab escaped — a newer entry corrupted rather than preserved.
    let fields: Vec<&str> = line.split('\t').collect();
    let (place, saved_at, progress, path) = match fields[..] {
        // The old form, where the third IS the path.
        [place, saved_at, path] => (place, saved_at, None, path),
        [place, saved_at, permille, words, path] => {
            (place, saved_at, Some((permille, words)), path)
        }
        _ => return None,
    };
    if path.is_empty() {
        return None;
    }
    Some(Entry {
        place: Place::decode(place)?,
        saved_at: saved_at.parse().ok()?,
        permille: progress.and_then(|(p, _)| p.parse().ok()),
        words: progress.and_then(|(_, w)| w.parse().ok()),
        path: unescape_path(path),
    })
}

/// Every remembered document, most recently read first.
///
/// The home screen's "continue reading" list. Entries whose file has gone
/// are the caller's to drop — this layer does not touch the disk to check.
#[must_use]
pub fn recent_in(dir: &Path) -> Vec<Entry> {
    let Ok(bytes) = std::fs::read(dir.join("positions")) else {
        return Vec::new();
    };
    let mut v: Vec<Entry> = parse(&bytes).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.saved_at));
    v
}

/// The canonical key for a file: symlink-resolved when possible, so the same
/// document reached by two spellings shares one position. A path, not a
/// string, so two names that differ only in bytes that are not UTF-8 stay
/// two documents; [`escape_path`] spells it for the file.
fn key(file: &Path) -> PathBuf {
    std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf())
}

/// Seconds since the epoch, now.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The saved place for `file`, if any. A missing directory, file, or entry
/// is `None` — never an error.
#[must_use]
pub fn load_position_in(dir: &Path, file: &Path) -> Option<Place> {
    let bytes = std::fs::read(dir.join("positions")).ok()?;
    let k = key(file);
    parse(&bytes).find(|e| e.path == k).map(|e| e.place)
}

/// Upsert `file`'s place; `None` removes the entry (the caller passes it at
/// the top of the file, which is the default — remembering it is noise).
/// Keeps the newest `CAP` entries by save time, and always the one being
/// saved.
pub fn save_position_in(
    dir: &Path,
    file: &Path,
    place: Option<Place>,
    permille: u16,
    words: u32,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("positions");
    let existing = read_state(&path)?;
    let k = key(file);
    let now = now();
    let mut entries: Vec<Entry> = Vec::new();
    // A line this version cannot read is kept as it is: most likely a newer
    // carrel's entry, in a form not invented yet, or a line a torn write left
    // half in bytes that are not UTF-8. Rebuilding the file from what parsed
    // is how 2026.10.21 discards every position a later carrel writes; this
    // version does not do the same to the next one. Written after the
    // readable entries, in the room the cap leaves.
    let mut unread: Vec<&[u8]> = Vec::new();
    for line in lines(&existing) {
        match std::str::from_utf8(line).ok().and_then(parse_line) {
            Some(e) if e.path == k => {}
            // A save time ahead of the clock — a clock that ran fast, or a
            // file from another machine — would outrank every save made now,
            // and with the file full the entry being saved sorted last and
            // was truncated away. Nothing is saved in the future.
            Some(e) => entries.push(Entry {
                saved_at: e.saved_at.min(now),
                ..e
            }),
            None => unread.push(line),
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.saved_at));
    let fresh = place.map(|place| Entry {
        place,
        saved_at: now,
        permille: Some(permille),
        words: Some(words),
        path: k,
    });
    // The cap evicts the oldest OTHER document: the one just read is the
    // one entry that must survive it, so it is placed first by hand rather
    // than trusted to the sort.
    entries.truncate(CAP - usize::from(fresh.is_some()));
    entries.splice(0..0, fresh);
    let mut out = String::new();
    for e in &entries {
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            e.place.encode(),
            e.saved_at,
            e.permille.unwrap_or(0),
            e.words.unwrap_or(0),
            escape_path(&e.path)
        );
    }
    let mut out = out.into_bytes();
    // After the readable entries, so a reader that takes the first line for
    // a document finds the one written now.
    for line in unread.iter().take(CAP - entries.len()) {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    write_atomic(&path, &out)
}

/// The bookmarks for `file`, in the order they were saved. Never an error.
/// A field this version cannot read is skipped, not the whole list.
///
/// Kept in their own file rather than as another column of `positions`:
/// a position is one number that is always overwritten, a bookmark list is
/// many and is edited. Same key function, so the two agree about what "the
/// same document reached two ways" means.
#[must_use]
pub fn load_marks_in(dir: &Path, file: &Path) -> Vec<Place> {
    let Ok(bytes) = std::fs::read(dir.join("bookmarks")) else {
        return Vec::new();
    };
    // Compared escaped, so one `escape_path` answers the whole file rather
    // than one unescape per line.
    let k = escape_path(&key(file));
    for line in lines(&bytes) {
        let Some(marks) = marks_of(line, &k) else {
            continue;
        };
        let mut out: Vec<Place> = Vec::new();
        for m in marks.split(',').filter_map(Place::decode) {
            if !out.contains(&m) {
                out.push(m);
            }
        }
        return out;
    }
    Vec::new()
}

/// The marks field of `line` if it is document `k`'s line, in the form this
/// version writes: exactly two fields. The path is escaped, so a raw tab
/// after the marks means a newer carrel's form, which is that carrel's to
/// read and this one's to leave alone — taking it for ours read its extra
/// field as one more unreadable mark and wrote it back inside the marks.
fn marks_of<'a>(line: &'a [u8], k: &str) -> Option<&'a str> {
    let (path, marks) = std::str::from_utf8(line).ok()?.split_once('\t')?;
    (path == k && !marks.contains('\t')).then_some(marks)
}

/// Replace `file`'s bookmarks. An empty list removes the entry.
///
/// **Path first here**, unlike `positions`, and the ordering is not what
/// makes the line safe. An earlier comment here argued the marks had to come
/// last because they are "the tab-safe field"; that is backwards. Whichever
/// field comes last is the one allowed to contain the separator, so putting
/// the marks there made the PATH the field that must not contain a tab — and
/// Unix paths may. `escape_field` is what makes the line safe, in both
/// files; the ordering is now free, and stays as it is only because changing
/// it would orphan every bookmark already on disk.
///
/// Keeps the newest `MARK_CAP` documents. Unlike `positions` there is no
/// timestamp to sort on, so "newest" is file order: this function appends the
/// document it just wrote and drops from the front, which is oldest-first as
/// long as every write comes through here.
pub fn save_marks_in(dir: &Path, file: &Path, marks: &[Place]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("bookmarks");
    let existing = read_state(&path)?;
    let k = escape_path(&key(file));
    // Every other line verbatim, as bytes: a line this version cannot read
    // — another document's in a newer form, or one a torn write left in
    // bytes that are not UTF-8 — is not this function's to drop.
    let mut kept: Vec<&[u8]> = lines(&existing)
        .filter(|l| marks_of(l, &k).is_none())
        .collect();
    let unread: Vec<&str> = lines(&existing)
        .filter_map(|l| marks_of(l, &k))
        .flat_map(|marks| marks.split(','))
        .filter(|m| !m.is_empty() && Place::decode(m).is_none())
        .collect();
    // Room for the line about to be appended, so the cap is a cap on the file
    // and not on the file plus one.
    let room = if marks.is_empty() && unread.is_empty() {
        MARK_CAP
    } else {
        MARK_CAP - 1
    };
    if kept.len() > room {
        kept.drain(..kept.len() - room);
    }
    let mut out: Vec<u8> = Vec::new();
    for line in kept {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    let mut joined: Vec<String> = Vec::with_capacity(marks.len());
    for m in marks {
        let m = m.encode();
        if !joined.contains(&m) {
            joined.push(m);
        }
    }
    // A mark this version cannot read stays on the line: it is the reader's,
    // set by a newer carrel, and nothing here can show it or clear it.
    for m in unread {
        if !joined.iter().any(|j| j == m) {
            joined.push(m.to_string());
        }
    }
    if !joined.is_empty() {
        out.extend_from_slice(format!("{k}\t{}\n", joined.join(",")).as_bytes());
    }
    write_atomic(&path, &out)
}

/// The same cap again, for collapsed sections.
const FOLD_CAP: usize = 500;

/// The sections of `file` that were left collapsed, as the keys
/// `App::fold_keys` wrote. Never an error.
///
/// A file of its own, like `bookmarks` and for its reason: a list that is
/// edited, beside a position that is only ever overwritten. One document
/// per line, the path first, then one escaped key per TAB-separated field.
#[must_use]
pub fn load_folds_in(dir: &Path, file: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(dir.join("folds")) else {
        return Vec::new();
    };
    let k = escape_path(&key(file));
    lines(&bytes)
        .find_map(|line| {
            let mut fields = std::str::from_utf8(line).ok()?.split('\t');
            (fields.next()? == k).then(|| fields.map(unescape_field).collect())
        })
        .unwrap_or_default()
}

/// Whether `line` is document `k`'s. A folds line has no fixed field
/// count — one key per field — so unlike the other two files there is no
/// "more fields than this version writes" to set aside: the path is all
/// that decides it.
fn is_folds_of(line: &[u8], k: &str) -> bool {
    line.split(|&b| b == b'\t').next() == Some(k.as_bytes())
}

/// Replace `file`'s collapsed sections. An empty list removes the entry, so
/// a document left fully expanded costs the file nothing.
pub fn save_folds_in(dir: &Path, file: &Path, keys: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("folds");
    let existing = read_state(&path)?;
    let k = escape_path(&key(file));
    // Every other line verbatim, as bytes, for the reason `save_marks_in`
    // gives.
    let mut kept: Vec<&[u8]> = lines(&existing).filter(|l| !is_folds_of(l, &k)).collect();
    if keys.is_empty() && kept.len() == lines(&existing).count() {
        return Ok(()); // nothing to remove and nothing to add
    }
    let room = if keys.is_empty() {
        FOLD_CAP
    } else {
        FOLD_CAP - 1
    };
    if kept.len() > room {
        kept.drain(..kept.len() - room);
    }
    let mut out: Vec<u8> = Vec::new();
    for line in kept {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    if !keys.is_empty() {
        out.extend_from_slice(k.as_bytes());
        for key in keys {
            out.push(b'\t');
            out.extend_from_slice(escape_field(key).as_bytes());
        }
        out.push(b'\n');
    }
    write_atomic(&path, &out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_kept_place_counts_repeated_paragraphs_and_a_gone_copy_keeps_its_source_place() {
        use carrel_core::{DocByte, Document};

        let old = Document::parse(
            "# Before\n\nRepeated café.\n\nBetween.\n\nRepeated café.\n\nBetween again.\n\nRepeated café.\n",
        );
        let new = Document::parse(
            "# A longer introduction\n\nRepeated café.\n\nDifferent middle.\n\nRepeated café.\n",
        );
        let starts = |doc: &Document| {
            doc.text
                .match_indices("Repeated café.")
                .map(|(at, _)| at as u32)
                .collect::<Vec<_>>()
        };
        let old_starts = starts(&old);
        let new_starts = starts(&new);
        assert_eq!(old_starts.len(), 3);
        assert_eq!(new_starts.len(), 2);
        for (i, &start) in old_starts.iter().enumerate() {
            for offset in [-1, 0, 2] {
                let byte = DocByte(start.checked_add_signed(offset).unwrap());
                let place = Place::kept(&old, byte);
                assert_eq!(place.resolve(&old), byte);
                // The third copy is gone: its place is its source place,
                // not the last copy left, which can be anywhere above.
                let want = if i < 2 {
                    new_starts[i].checked_add_signed(offset).unwrap()
                } else {
                    Place::of(&old, byte).resolve(&new).0
                };
                assert_eq!(place.resolve(&new).0, want, "copy {i} offset {offset}");
            }
        }
        let gone = Document::parse("No matching paragraph.\n");
        for start in old_starts {
            assert_eq!(
                Place::kept(&old, DocByte(start)).resolve(&gone),
                Place::of(&old, DocByte(start)).resolve(&gone)
            );
        }
    }

    use super::*;

    /// A place in the old spelling, which every test below that is about
    /// the FILE rather than the coordinate uses.
    #[allow(non_snake_case)]
    fn P(d: u32) -> Place {
        Place::Legacy(d)
    }

    #[test]
    fn a_place_is_spelled_with_its_sign_and_reads_back() {
        for (p, text) in [
            (
                Place::Src {
                    src: 1024,
                    delta: 16,
                    block: None,
                },
                "s1024+16",
            ),
            (
                Place::Src {
                    src: 7,
                    delta: -5,
                    block: None,
                },
                "s7-5",
            ),
            (
                Place::Src {
                    src: 0,
                    delta: 0,
                    block: None,
                },
                "s0+0",
            ),
            (Place::Legacy(1234), "1234"),
            (
                Place::Src {
                    src: 40,
                    delta: 2,
                    block: Some(Kept {
                        hash: 0xab,
                        nth: 3,
                        off: -1,
                        prev: None,
                    }),
                },
                "s40+2@00000000000000ab.3-1",
            ),
            (
                Place::Src {
                    src: 0,
                    delta: 0,
                    block: Some(Kept {
                        hash: u64::MAX,
                        nth: 0,
                        off: 12,
                        prev: None,
                    }),
                },
                "s0+0@ffffffffffffffff.0+12",
            ),
            (
                Place::Src {
                    src: 7,
                    delta: -3,
                    block: Some(Kept {
                        hash: 0xab,
                        nth: 1,
                        off: 0,
                        prev: Some(0x1234),
                    }),
                },
                "s7-3@00000000000000ab.1+0^0000000000001234",
            ),
        ] {
            assert_eq!(p.encode(), text);
            assert_eq!(Place::decode(text), Some(p), "{text}");
        }
        // Nothing this version did not write is read as a place: a newer
        // carrel's field is skipped rather than taken for a position.
        for junk in [
            "",
            "s",
            "s12",
            "s+1",
            "sx+1",
            "s1+x",
            "v2:9",
            "-4",
            "12.5",
            "s1+0@",
            "s1+0@ab.0+1",
            "s1+0@00000000000000ab",
            "s1+0@00000000000000ab.x+1",
            "s1+0@00000000000000ab.0",
            "s1+0@zzzzzzzzzzzzzzzz.0+1",
        ] {
            assert_eq!(Place::decode(junk), None, "{junk:?}");
        }
    }

    /// The files carrel 2026.10.21 and earlier wrote go on reading, line by
    /// line beside the new spelling, and a mark this version cannot read
    /// costs only itself.
    #[test]
    fn files_written_by_older_and_newer_carrels_read_line_by_line() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.md");
        let new = dir.path().join("new.md");
        let future = dir.path().join("future.md");
        for f in [&old, &new, &future] {
            std::fs::write(f, "x").unwrap();
        }
        let k = |f: &Path| escape_path(&key(f));
        std::fs::write(
            dir.path().join("positions"),
            format!(
                "300\t5\t10\t20\t{}\ns40+2\t6\t10\t20\t{}\nv9:1\t7\t10\t20\t{}\n",
                k(&old),
                k(&new),
                k(&future)
            ),
        )
        .unwrap();
        assert_eq!(load_position_in(dir.path(), &old), Some(Place::Legacy(300)));
        assert_eq!(
            load_position_in(dir.path(), &new),
            Some(Place::Src {
                src: 40,
                delta: 2,
                block: None
            })
        );
        assert_eq!(load_position_in(dir.path(), &future), None);

        std::fs::write(
            dir.path().join("bookmarks"),
            format!("{}\t5,s40-1,z9,s40-1\n", k(&old)),
        )
        .unwrap();
        assert_eq!(
            load_marks_in(dir.path(), &old),
            [
                Place::Legacy(5),
                Place::Src {
                    src: 40,
                    delta: -1,
                    block: None
                }
            ],
            "the unreadable one dropped, the repeat folded"
        );
    }

    /// A line a newer carrel wrote, in a form this one cannot read, survives
    /// this one saving another document's position — and a bookmark this one
    /// cannot read survives it saving that document's bookmarks.
    #[test]
    fn what_this_version_cannot_read_it_writes_back() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        std::fs::write(&a, "x").unwrap();
        let future = "v9:1\t7\t10\t20\t/x/future.md";
        std::fs::write(dir.path().join("positions"), format!("{future}\n")).unwrap();
        save_position_in(dir.path(), &a, Some(P(5)), 0, 0).unwrap();
        save_position_in(dir.path(), &a, Some(P(6)), 0, 0).unwrap();
        let text = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(text.lines().filter(|l| *l == future).count(), 1, "{text:?}");
        assert!(
            text.lines().next().unwrap().starts_with('6'),
            "readable first: {text:?}"
        );
        assert_eq!(load_position_in(dir.path(), &a), Some(P(6)));

        let k = escape_path(&key(&a));
        std::fs::write(dir.path().join("bookmarks"), format!("{k}\t5,w2:9\n")).unwrap();
        save_marks_in(dir.path(), &a, &[P(5), P(8)]).unwrap();
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text, format!("{k}\t5,8,w2:9\n"));
        save_marks_in(dir.path(), &a, &[]).unwrap();
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text, format!("{k}\tw2:9\n"), "clearing what it can see");
    }

    /// Wherever a place lands — a file shortened since, a display byte from
    /// an older core that falls inside a character now — it lands inside
    /// the text and on a character boundary.
    #[test]
    fn a_place_always_resolves_inside_the_text_on_a_boundary() {
        let doc = carrel_core::Document::parse("“Quoted” — naïve\n");
        let len = doc.text.len() as u32;
        for p in [
            Place::Legacy(1),
            Place::Legacy(u32::MAX),
            Place::Src {
                src: 9999,
                delta: 50,
                block: None,
            },
            Place::Src {
                src: 0,
                delta: -50,
                block: None,
            },
            Place::Src {
                src: 2,
                delta: 1,
                block: None,
            },
        ] {
            let at = p.resolve(&doc).0;
            assert!(at < len, "{p:?} -> {at}");
            assert!(doc.text.is_char_boundary(at as usize), "{p:?} -> {at}");
        }
    }

    #[test]
    fn round_trips_bookmarks_and_keeps_other_documents() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "y").unwrap();
        assert_eq!(
            load_marks_in(dir.path(), &a),
            [] as [crate::state::Place; 0]
        );

        save_marks_in(dir.path(), &a, &[P(300), P(100), P(100)]).unwrap();
        assert_eq!(
            load_marks_in(dir.path(), &a),
            vec![P(300), P(100)],
            "deduped; the reader sorts what it resolves"
        );

        save_marks_in(dir.path(), &b, &[P(7)]).unwrap();
        assert_eq!(
            load_marks_in(dir.path(), &a),
            vec![P(300), P(100)],
            "a survived"
        );
        assert_eq!(load_marks_in(dir.path(), &b), vec![P(7)]);

        save_marks_in(dir.path(), &a, &[]).unwrap();
        assert!(load_marks_in(dir.path(), &a).is_empty(), "cleared");
        assert_eq!(
            load_marks_in(dir.path(), &b),
            vec![P(7)],
            "b still survived"
        );
    }

    #[test]
    fn round_trips_a_position() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("doc.md");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(load_position_in(dir.path(), &f), None, "nothing saved yet");
        save_position_in(dir.path(), &f, Some(P(1234)), 0, 0).unwrap();
        assert_eq!(load_position_in(dir.path(), &f), Some(P(1234)));
        save_position_in(dir.path(), &f, Some(P(99)), 0, 0).unwrap();
        assert_eq!(
            load_position_in(dir.path(), &f),
            Some(P(99)),
            "upsert, not append"
        );
    }

    #[test]
    fn no_place_deletes_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("doc.md");
        std::fs::write(&f, "x").unwrap();
        save_position_in(dir.path(), &f, Some(P(1234)), 0, 0).unwrap();
        save_position_in(dir.path(), &f, None, 0, 0).unwrap();
        assert_eq!(
            load_position_in(dir.path(), &f),
            None,
            "scrolled back to top"
        );
    }

    #[test]
    fn two_files_keep_independent_positions() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "x").unwrap();
        save_position_in(dir.path(), &a, Some(P(10)), 0, 0).unwrap();
        save_position_in(dir.path(), &b, Some(P(20)), 0, 0).unwrap();
        assert_eq!(load_position_in(dir.path(), &a), Some(P(10)));
        assert_eq!(load_position_in(dir.path(), &b), Some(P(20)));
    }

    #[test]
    fn the_cap_evicts_and_the_file_never_exceeds_it() {
        let dir = tempfile::tempdir().unwrap();
        // Paths need not exist: canonicalize falls back to the spelling.
        for i in 0..(CAP + 10) {
            let f = dir.path().join(format!("f{i}.md"));
            save_position_in(dir.path(), &f, Some(P(1)), 0, 0).unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(text.lines().count(), CAP);
    }

    #[test]
    fn malformed_lines_are_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("positions"), "garbage\n12\tnotanum\t/x\n").unwrap();
        let f = dir.path().join("doc.md");
        assert_eq!(load_position_in(dir.path(), &f), None);
    }

    #[test]
    fn collapsed_sections_round_trip_and_an_empty_list_removes_the_entry() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let b = d.path().join("b\tweird\nname.md");
        let keys = vec!["h:setup".to_string(), "d:0:A tab\there".to_string()];
        save_folds_in(d.path(), &a, &keys).unwrap();
        save_folds_in(d.path(), &b, &["h:x".to_string()]).unwrap();
        assert_eq!(load_folds_in(d.path(), &a), keys);
        assert_eq!(
            load_folds_in(d.path(), &b),
            ["h:x"],
            "a hostile path is one field"
        );

        save_folds_in(d.path(), &a, &[]).unwrap();
        assert_eq!(load_folds_in(d.path(), &a), [] as [String; 0]);
        assert_eq!(
            load_folds_in(d.path(), &b),
            ["h:x"],
            "the other document stays"
        );
        assert_eq!(
            load_folds_in(d.path(), &d.path().join("never.md")),
            [] as [String; 0]
        );
        assert_eq!(
            load_folds_in(&d.path().join("no-such-dir"), &a),
            [] as [String; 0]
        );
    }

    #[test]
    fn a_document_with_nothing_collapsed_writes_no_file() {
        let d = tempfile::tempdir().unwrap();
        save_folds_in(d.path(), &d.path().join("a.md"), &[]).unwrap();
        assert!(!d.path().join("folds").exists());
    }

    #[test]
    fn a_missing_directory_is_not_an_error_on_load() {
        assert_eq!(
            load_position_in(Path::new("/nonexistent/xyzzy"), Path::new("/tmp/x.md")),
            None
        );
    }

    /// A path may contain a TAB, a newline or a backslash — Unix forbids only
    /// `/` and NUL. Unescaped, the tab splits the record inside the path and
    /// the newline splits one record into two, so nothing ever matches again.
    #[test]
    fn a_path_with_a_tab_newline_or_backslash_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["with\ttab.md", "with\nnewline.md", "with\\slash.md"] {
            let f = dir.path().join(name);
            std::fs::write(&f, "x").unwrap();
            let at = Place::Src {
                src: 42,
                delta: -3,
                block: None,
            };
            save_position_in(dir.path(), &f, Some(at), 500, 100).unwrap();
            assert_eq!(load_position_in(dir.path(), &f), Some(at), "{name:?}");
            let marks = [
                Place::Src {
                    src: 5,
                    delta: 0,
                    block: None,
                },
                Place::Src {
                    src: 9,
                    delta: 2,
                    block: None,
                },
            ];
            save_marks_in(dir.path(), &f, &marks).unwrap();
            assert_eq!(load_marks_in(dir.path(), &f), marks, "{name:?}");
        }
        // One record per document, not one per field the path was split into.
        let positions = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(positions.lines().count(), 3, "{positions:?}");
        let bookmarks = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(bookmarks.lines().count(), 3, "{bookmarks:?}");
    }

    /// The failure that has no cap to stop it: `save_marks_in` decides which
    /// line is its own with the same comparison `load_marks_in` uses, so a
    /// path it cannot recognise is a line it never replaces — one more entry
    /// per save, forever.
    #[test]
    fn saving_the_same_document_repeatedly_writes_exactly_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a\tb.md");
        std::fs::write(&f, "x").unwrap();
        for i in 1..=20u32 {
            save_marks_in(dir.path(), &f, &[P(i)]).unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert_eq!(load_marks_in(dir.path(), &f), vec![P(20)]);
    }

    #[test]
    fn the_bookmark_cap_evicts_rather_than_growing_without_bound() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MARK_CAP + 10) {
            let f = dir.path().join(format!("f{i}.md"));
            save_marks_in(dir.path(), &f, &[P(1)]).unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text.lines().count(), MARK_CAP);
        // The oldest go; the newest is certainly still there.
        let newest = dir.path().join(format!("f{}.md", MARK_CAP + 9));
        assert_eq!(load_marks_in(dir.path(), &newest), vec![P(1)]);
    }

    /// The reason `fs::write` is not good enough: it truncates the target and
    /// then writes it back, so anything reading concurrently — a second
    /// carrel, or a crash's postmortem — can see a half-written or empty file
    /// where a full one was. A `rename` onto the target cannot be observed
    /// half-done.
    #[test]
    fn a_concurrent_reader_never_sees_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let seed = dir.path().join("seed.md");
        for i in 0..CAP {
            save_position_in(
                dir.path(),
                &dir.path().join(format!("f{i}.md")),
                Some(P(1)),
                0,
                0,
            )
            .unwrap();
        }
        std::fs::write(&seed, "x").unwrap();
        let positions = dir.path().join("positions");
        let full = std::fs::read_to_string(&positions).unwrap().lines().count();
        assert_eq!(full, CAP, "the file under test is a big one");

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (p, stop) = (positions.clone(), std::sync::Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut worst = usize::MAX;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        worst = worst.min(t.lines().count());
                    }
                }
                worst
            })
        };
        for i in 0..200u32 {
            let f = dir.path().join(format!("f{}.md", i % 50));
            save_position_in(dir.path(), &f, Some(P(i + 1)), 0, 0).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let worst = reader.join().unwrap();
        assert_eq!(
            worst, full,
            "a reader saw a {worst}-line file where {full} lines were on disk",
        );
    }

    /// One byte that is not UTF-8 — a torn write from a release that still
    /// truncated in place — made `read_to_string` fail, and the next save
    /// wrote back only its own entry: every other document lost, in all
    /// three files. Now the bad line costs only itself, and is kept.
    #[test]
    fn a_line_that_is_not_utf8_costs_only_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.md"), dir.path().join("b.md"));
        let torn: &[u8] = b"s1+0\t5\t0\t0\t/tmp/\xff\xfe torn";

        save_position_in(dir.path(), &a, Some(P(3)), 100, 10).unwrap();
        let mut t = std::fs::read(dir.path().join("positions")).unwrap();
        t.extend_from_slice(torn);
        t.push(b'\n');
        std::fs::write(dir.path().join("positions"), &t).unwrap();
        assert_eq!(load_position_in(dir.path(), &a), Some(P(3)), "load too");
        save_position_in(dir.path(), &b, Some(P(5)), 100, 10).unwrap();
        assert_eq!(load_position_in(dir.path(), &a), Some(P(3)));
        assert_eq!(load_position_in(dir.path(), &b), Some(P(5)));
        let t = std::fs::read(dir.path().join("positions")).unwrap();
        assert!(t.split(|&c| c == b'\n').any(|l| l == torn), "kept verbatim");
        assert_eq!(recent_in(dir.path()).len(), 2);

        save_marks_in(dir.path(), &a, &[P(7)]).unwrap();
        let mut t = std::fs::read(dir.path().join("bookmarks")).unwrap();
        t.extend_from_slice(b"/x/\xff\t1,2\n");
        std::fs::write(dir.path().join("bookmarks"), &t).unwrap();
        assert_eq!(load_marks_in(dir.path(), &a), [P(7)], "load too");
        save_marks_in(dir.path(), &b, &[P(8)]).unwrap();
        assert_eq!(load_marks_in(dir.path(), &a), [P(7)]);
        let t = std::fs::read(dir.path().join("bookmarks")).unwrap();
        assert!(t.split(|&c| c == b'\n').any(|l| l == b"/x/\xff\t1,2"));

        save_folds_in(dir.path(), &a, &["h:x".into()]).unwrap();
        let mut t = std::fs::read(dir.path().join("folds")).unwrap();
        t.extend_from_slice(b"/x/\xff\th:y\n");
        std::fs::write(dir.path().join("folds"), &t).unwrap();
        assert_eq!(load_folds_in(dir.path(), &a), ["h:x"], "load too");
        save_folds_in(dir.path(), &b, &["h:z".into()]).unwrap();
        assert_eq!(load_folds_in(dir.path(), &a), ["h:x"]);
        let t = std::fs::read(dir.path().join("folds")).unwrap();
        assert!(t.split(|&c| c == b'\n').any(|l| l == b"/x/\xff\th:y"));
    }

    /// A file that cannot be read at all is not replaced by the one entry
    /// being saved: the save refuses, and the file is as it was.
    #[cfg(unix)]
    #[test]
    fn a_state_file_it_cannot_read_is_never_overwritten() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        for name in ["positions", "bookmarks", "folds"] {
            let p = dir.path().join(name);
            std::fs::write(&p, "precious\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o200)).unwrap();
            if std::fs::read(&p).is_ok() {
                return; // running as root: permissions cannot stop a read
            }
        }
        assert!(save_position_in(dir.path(), &a, Some(P(1)), 0, 0).is_err());
        assert!(save_marks_in(dir.path(), &a, &[P(1)]).is_err());
        assert!(save_folds_in(dir.path(), &a, &["h:x".into()]).is_err());
        for name in ["positions", "bookmarks", "folds"] {
            let p = dir.path().join(name);
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(std::fs::read_to_string(&p).unwrap(), "precious\n", "{name}");
        }
    }

    /// `rename` onto a symlink replaces the link: a stow-managed state or
    /// config file became a regular file and the dotfiles copy stopped
    /// changing. The write goes through the link to its target.
    #[cfg(unix)]
    #[test]
    fn writing_through_a_symlink_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let dots = dir.path().join("dotfiles");
        std::fs::create_dir(&dots).unwrap();
        let target = dots.join("positions");
        std::fs::write(&target, "").unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        std::os::unix::fs::symlink(&target, state.join("positions")).unwrap();
        let a = dir.path().join("a.md");
        save_position_in(&state, &a, Some(P(9)), 0, 0).unwrap();
        let link = std::fs::symlink_metadata(state.join("positions")).unwrap();
        assert!(link.file_type().is_symlink(), "the link survived");
        assert!(std::fs::read_to_string(&target).unwrap().starts_with("9\t"));
        assert_eq!(load_position_in(&state, &a), Some(P(9)));
        // A dangling link is written where it points.
        let gone = dots.join("bookmarks");
        std::os::unix::fs::symlink(&gone, state.join("bookmarks")).unwrap();
        save_marks_in(&state, &a, &[P(4)]).unwrap();
        assert!(
            std::fs::symlink_metadata(state.join("bookmarks"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(load_marks_in(&state, &a), [P(4)]);
        assert!(gone.is_file());
    }

    /// `\xff.md` and `\xfe.md` are two files, and `display()` made them one
    /// key, `\u{fffd}.md`, which is neither — so they shared a position and
    /// the home screen offered a path that does not exist.
    #[cfg(unix)]
    #[test]
    fn paths_that_are_not_utf8_are_their_own_documents() {
        use std::os::unix::ffi::OsStrExt as _;
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(std::ffi::OsStr::from_bytes(b"\xff.md"));
        let b = dir.path().join(std::ffi::OsStr::from_bytes(b"\xfe.md"));
        let c = dir
            .path()
            .join(std::ffi::OsStr::from_bytes(b"lit\\xff\t\xff.md"));
        for f in [&a, &b, &c] {
            std::fs::write(f, "x").unwrap();
        }
        save_position_in(dir.path(), &a, Some(P(7)), 500, 10).unwrap();
        assert_eq!(load_position_in(dir.path(), &b), None);
        save_position_in(dir.path(), &c, Some(P(8)), 500, 10).unwrap();
        assert_eq!(load_position_in(dir.path(), &a), Some(P(7)));
        assert_eq!(load_position_in(dir.path(), &c), Some(P(8)));
        let recent: Vec<PathBuf> = recent_in(dir.path()).into_iter().map(|e| e.path).collect();
        assert!(
            recent.contains(&key(&a)) && recent.contains(&key(&c)),
            "{recent:?}"
        );
        assert!(recent.iter().all(|p| p.is_file()), "{recent:?}");

        save_marks_in(dir.path(), &a, &[P(1)]).unwrap();
        assert!(load_marks_in(dir.path(), &b).is_empty());
        save_folds_in(dir.path(), &a, &["h:x".into()]).unwrap();
        assert!(load_folds_in(dir.path(), &b).is_empty());
        assert_eq!(load_folds_in(dir.path(), &a), ["h:x"]);

        // A UTF-8 path escapes exactly as it always did, so old lines match.
        for s in ["/plain/é.md", "/a\tb\\c\nd.md", "/lit\\xff.md"] {
            assert_eq!(escape_path(Path::new(s)), escape_field(s));
            assert_eq!(unescape_path(&escape_path(Path::new(s))), Path::new(s));
        }
        for raw in [&b"\xff"[..], b"a\\xzz\xfe\\", b"\\x4"] {
            let p = Path::new(std::ffi::OsStr::from_bytes(raw));
            assert_eq!(unescape_path(&escape_path(p)), p, "{raw:?}");
        }
    }

    /// Entries saved by a clock that ran ahead outranked every save made
    /// now, so with the file full the entry being saved sorted last and was
    /// cut by the cap: silently not remembered.
    #[test]
    fn the_entry_being_saved_always_survives_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = String::new();
        for i in 0..CAP {
            use std::fmt::Write as _;
            let _ = writeln!(t, "s0+5\t{}\t100\t10\t/old/{i}.md", 4_000_000_000 + i);
        }
        std::fs::write(dir.path().join("positions"), t).unwrap();
        let doc = dir.path().join("doc.md");
        save_position_in(dir.path(), &doc, Some(P(7)), 500, 10).unwrap();
        assert_eq!(load_position_in(dir.path(), &doc), Some(P(7)));
        assert_eq!(
            recent_in(dir.path())[0].path,
            key(&doc),
            "and is the most recent"
        );
        let text = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(text.lines().count(), CAP);
    }

    /// A line with more fields than this version writes is a newer carrel's,
    /// and goes back exactly as it came — `splitn(5)` read the extra field
    /// into the path and wrote the line back with its tab escaped.
    #[test]
    fn a_line_with_more_fields_than_this_version_writes_is_kept_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("doc.md");
        let newer = "s10+0\t1794000000\t500\t100\tnewfield=1\t/docs/plan.md";
        std::fs::write(dir.path().join("positions"), format!("{newer}\n")).unwrap();
        assert!(recent_in(dir.path()).is_empty(), "not misread");
        save_position_in(dir.path(), &doc, Some(P(7)), 500, 10).unwrap();
        let text = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(text.lines().last(), Some(newer), "{text:?}");

        let k = escape_path(&key(&doc));
        let newer = format!("{k}\t5,6\tcolor=blue");
        std::fs::write(dir.path().join("bookmarks"), format!("{newer}\n")).unwrap();
        assert!(load_marks_in(dir.path(), &doc).is_empty(), "not misread");
        save_marks_in(dir.path(), &doc, &[P(9)]).unwrap();
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text, format!("{newer}\n{k}\t9\n"));
        assert_eq!(load_marks_in(dir.path(), &doc), [P(9)]);
    }
}
