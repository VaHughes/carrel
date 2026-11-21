//! Image loading, off the paint path. **NO RATATUI** — rule 6.
//!
//! This module owns decode and dimensions; protocols and widgets live in
//! `render.rs`/`main.rs`, the only files that may see `ratatui-image` types.
//! The `image` crate is not a UI crate: it turns bytes into pixels and knows
//! nothing about terminals.
//!
//! # Hardening
//!
//! Image decoders are the classic attack surface for untrusted files, and a
//! markdown document can point at any file the user can read. Memory-safe
//! decoding still wants bomb guards: a file-size pre-check plus
//! [`image::Limits`] on dimensions and allocation.
//!
//! Those caps are per *image*, and a document is many images. A 265 KB PNG
//! that decodes to 8000×8000 RGBA is inside every one of them, yet each
//! `![a](big.png)` used to decode and keep its own 256 MB copy — twenty
//! references aborted the process with the terminal still in raw mode. So the
//! worker decodes each file once, shrinks it to what a terminal can show, and
//! spends from one budget for the whole document ([`DecodeCaps`]).
//!
//! An image outside the folder carrel was pointed at is not opened
//! ([`contain`]) — the same rule, through the same function, that makes a
//! link out of the folder ask first — until the reader, shown its path in the
//! lightbox, answers; that consents to that one file for the session
//! (`App::image_consent`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use carrel_core::{BlockIdx, Document, NodeKind};

/// Files larger than this are not decoded. A 64 MiB "figure" is a mistake.
pub const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Per-axis pixel cap fed to [`image::Limits`].
pub const MAX_IMAGE_AXIS_PX: u32 = 8192;
/// Decoder allocation cap fed to [`image::Limits`].
pub const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
/// Decoded pixels kept for one document, summed over every image block.
///
/// Counted per block, not per file: each block's display protocol owns its
/// own copy of the pixels, so that is what memory actually holds. Past it, a
/// block renders its alt text and the lightbox says why.
pub const MAX_DOCUMENT_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
/// The narrowest width an image is shrunk to, whatever the terminal says
/// now: room for the window to grow wider later without the picture
/// stopping short of the column it was laid out for.
pub const MIN_WIDTH_CEILING_PX: u32 = 2560;

/// What the decode worker may spend.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DecodeCaps {
    /// Images wider than this are shrunk to it at decode, aspect kept.
    pub max_width_px: u32,
    /// Total decoded bytes across every block of the document.
    pub budget_bytes: u64,
}

impl Default for DecodeCaps {
    fn default() -> Self {
        Self {
            max_width_px: MIN_WIDTH_CEILING_PX,
            budget_bytes: MAX_DOCUMENT_IMAGE_BYTES,
        }
    }
}

impl DecodeCaps {
    /// Caps for a terminal `text_px` pixels wide: nothing wider than the
    /// terminal can ever be painted, so nothing wider is kept.
    #[must_use]
    pub fn for_terminal(text_px: u32) -> Self {
        Self {
            max_width_px: text_px.max(MIN_WIDTH_CEILING_PX),
            ..Self::default()
        }
    }
}

/// What a block outside the folder says instead of its picture.
pub const OUTSIDE_FOLDER: &str = "outside this folder, so it was not opened";

/// The button a refused image's block wears, after its alt text. The whole
/// block is the button; this says so.
pub const SHOW_OUTSIDE: &str = "[outside this folder \u{2014} click to show]";
/// The short form, for a row with no room for the long one.
pub const SHOW_OUTSIDE_SHORT: &str = "[click to show]";

/// What decides whether the open document's images need planning again:
/// the file, the folder it is read within, and how many images outside
/// that folder the reader has chosen to see.
///
/// Keyed on the file alone, the verdict stuck when the file list moved and
/// the same document was opened again. After Backspace from `docs` to the
/// project, `../assets` images stayed refused until the file was touched;
/// after the picker narrowed the root from the project to `docs`, they
/// still rendered, which let a stale verdict walk past containment. The
/// consent count is there for the same reason: without it an answer on a
/// refused image changed nothing until the document was opened again.
#[must_use]
pub fn sync_key(app: &crate::app::App) -> SyncKey {
    (
        app.file.clone(),
        app.library_root().map(Path::to_path_buf),
        app.image_consent.len(),
    )
}

/// [`sync_key`]'s value. `App::image_consent` only grows, so its length
/// is a generation.
pub type SyncKey = (Option<PathBuf>, Option<PathBuf>, usize);

/// Plan the open document's images: forget every earlier verdict, refuse
/// what lies outside the library, and return what is left to decode.
///
/// Forgetting first matters when the same parse is planned under a new
/// root: a refusal recorded under the old one must not survive, and
/// neither may the dimensions of an image that is now outside.
pub fn plan(app: &mut crate::app::App) -> Vec<(BlockIdx, PathBuf)> {
    let had = !app.image_dims.is_empty();
    app.image_dims.clear();
    app.image_errors.clear();
    app.image_outside.clear();
    app.image_plan = app.image_plan.wrapping_add(1);
    if had {
        app.relayout();
    }
    let base = app.file.as_deref().and_then(Path::parent);
    let reqs = local_image_requests(&app.doc, base);
    // The links' containment rule, through the links' root: an image
    // outside the folder is never opened, and keeps its alt text — unless
    // the reader chose to see that very file (`App::image_consent`).
    let (mut reqs, outside) = contain(reqs, app.library_root());
    let mut refused = 0;
    for (block, canonical) in outside {
        if app.image_consent.contains(&canonical) {
            // The canonical path, not the spelling in the document: what
            // the reader consented to is what the decoder opens.
            reqs.push((block, canonical));
        } else {
            refused += 1;
            app.image_errors.insert(block, OUTSIDE_FOLDER.to_string());
            app.image_outside.insert(block, canonical);
        }
    }
    // Document order, so the image budget is spent the way it always was.
    reqs.sort_by_key(|(block, _)| *block);
    if refused > 0 && app.note.is_none() {
        app.note = Some(outside_note(refused));
    }
    reqs
}

/// Plan only what the reader has just consented to: every refused block
/// whose canonical path is now in `App::image_consent`, leaving every other
/// verdict, dimension and decoded picture where it is.
///
/// `plan` forgets everything first, so a consent re-planned with it dropped
/// every decoded image in the document to alt-text height and decoded it
/// again — a visible collapse and reflow for a click on one picture.
pub fn plan_consented(app: &mut crate::app::App) -> Vec<Request> {
    let mut reqs: Vec<Request> = app
        .image_outside
        .iter()
        .filter(|(_, path)| app.image_consent.contains(*path))
        .map(|(b, p)| (*b, p.clone()))
        .collect();
    reqs.sort_by_key(|(block, _)| *block);
    for (block, _) in &reqs {
        app.image_outside.remove(block);
        app.image_errors.remove(block);
    }
    // The verdicts changed, so an answer offered under the old ones is stale.
    app.image_plan = app.image_plan.wrapping_add(1);
    reqs
}

/// Where a refused image leads, said so the reader can judge it: relative
/// to `root` when that is the shorter spelling (`../outside/red.png`), else
/// absolute; and, when the document named it some other way — a symlink,
/// a `./` — both, as `link.png → ../outside/red.png`. Every name is passed
/// through [`crate::plain::shown`], so a control in a file name is escaped.
///
/// The lightbox put up only the absolute path, cut at the right edge: at
/// 60 columns the reader saw `/tmp/claude-1000/-home-…` and could answer
/// `[show]` without ever seeing which file (black-box test, 2026-11-14).
#[must_use]
pub fn outside_where(named: &str, canonical: &Path, root: Option<&Path>) -> String {
    let absolute = crate::plain::shown(canonical);
    let target = root
        .and_then(|r| r.canonicalize().ok())
        .map(|r| crate::plain::shown(&relative_to(canonical, &r)))
        .filter(|rel| rel.len() < absolute.len())
        .unwrap_or(absolute);
    let named = crate::plain::shown(Path::new(named));
    if named == target {
        target
    } else {
        format!("{named} \u{2192} {target}")
    }
}

/// `path` spelled from `base`, both absolute: `..` up to their common
/// ancestor, then down.
fn relative_to(path: &Path, base: &Path) -> PathBuf {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    out
}

/// `text` broken into rows of at most `width` cells, keeping every
/// character: at spaces where it can, inside a word where it must (a path
/// has no spaces, and every cell of it matters).
#[must_use]
pub fn wrap_cells(text: &str, width: u16) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split_inclusive(' ') {
        let w = carrel_core::display_width(word.trim_end());
        if !row.is_empty() && carrel_core::display_width(row.trim_end()) + w > width {
            rows.push(row.trim_end().to_string());
            row = String::new();
        }
        for g in word.graphemes(true) {
            let gw = carrel_core::display_width(g);
            if g != " " && carrel_core::display_width(&row) + gw > width {
                rows.push(row.trim_end().to_string());
                row = String::new();
            }
            row.push_str(g);
        }
    }
    if !row.trim_end().is_empty() {
        rows.push(row.trim_end().to_string());
    }
    rows
}

/// `text` in `width` cells keeping its END — `…` plus the tail — which is
/// how carrel shortens a path: the file name and its folder are the part
/// that says which file.
#[must_use]
pub fn keep_end(text: &str, width: u16) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if carrel_core::display_width(text) <= width {
        return text.to_string();
    }
    let budget = width.saturating_sub(1);
    let mut tail: Vec<&str> = Vec::new();
    let mut used = 0u16;
    for g in text.graphemes(true).rev() {
        let w = carrel_core::display_width(g);
        if used + w > budget {
            break;
        }
        used += w;
        tail.push(g);
    }
    tail.reverse();
    if width == 0 {
        return String::new();
    }
    format!("\u{2026}{}", tail.concat())
}

/// The lightbox's question over a refused image, in at most `rows` rows of
/// `width` cells, most important first to keep: where it leads (whole if
/// it fits, else its end), then what it is asking.
#[must_use]
pub fn outside_question(where_: &str, width: u16, rows: u16) -> Vec<String> {
    let ask = "is outside this folder \u{2014} Enter or [show] shows it for this session";
    let mut full = wrap_cells(where_, width);
    full.extend(wrap_cells(ask, width));
    if full.len() <= usize::from(rows) {
        return full;
    }
    let mut short = vec![keep_end(where_, width)];
    short.extend(wrap_cells(ask, width));
    short.truncate(usize::from(rows));
    short
}

/// "1 image outside this folder…", "2 images outside this folder…": the
/// note once said "2 images … so it was not opened".
#[must_use]
pub fn outside_note(n: usize) -> String {
    if n == 1 {
        format!("1 image {OUTSIDE_FOLDER}")
    } else {
        format!("{n} images outside this folder, so they were not opened")
    }
}

/// What the decode worker reports.
#[derive(Debug)]
pub enum ImageMsg {
    /// Decoded pixels. Dimensions come from the image itself.
    Decoded(BlockIdx, image::DynamicImage),
    Failed(BlockIdx, String),
}

/// The image blocks worth decoding: local paths only, resolved against the
/// document's own directory.
///
/// Remote URLs are **never fetched** — the reader sends nothing anywhere, and
/// their blocks simply render alt text. `data:` URIs are skipped for the same
/// reason `file://` is not special-cased: markdown images in documents worth
/// reading are relative paths.
#[must_use]
pub fn local_image_requests(doc: &Document, base: Option<&Path>) -> Vec<(BlockIdx, PathBuf)> {
    let mut out = Vec::new();
    for b in 0..doc.block_count() {
        let block = BlockIdx(b as u32);
        let NodeKind::Image { url } = &doc.node_for_block(block).kind else {
            continue;
        };
        if url.contains("://") || url.starts_with("mailto:") || url.starts_with("data:") {
            continue;
        }
        let path = Path::new(&**url);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match base {
                Some(dir) => dir.join(path),
                None => continue,
            }
        };
        out.push((block, resolved));
    }
    out
}

/// Split requests into those inside `root` and the blocks that leave it.
///
/// `local_image_requests` takes an absolute path or a `..` as readily as a
/// sibling, so `![x](../../.ssh/id.png)` in a downloaded README was opened and
/// decoded with no question asked, while the README promised carrel reads
/// only the folder you point it at. A link out of the folder waits for a
/// second Enter; an image is not opened and keeps its alt text, which is a
/// button opening the lightbox that asks (`App::image_consent`). `root` is
/// `App::library_root`, so the rule is the links' rule in every case: a
/// file opened directly is rooted at the folder you stand in or its own, a
/// pipe at the working directory, the home list at its root.
///
/// A refused block comes back with its CANONICAL path, which is what a
/// reader's consent names: a symlink and its target are one file, and a
/// symlink later pointed elsewhere is another.
#[must_use]
pub fn contain(
    reqs: Vec<(BlockIdx, PathBuf)>,
    root: Option<&Path>,
) -> (Vec<Request>, Vec<Request>) {
    let Some(root) = root else {
        return (reqs, Vec::new());
    };
    let (outside, inside): (Vec<_>, Vec<_>) = reqs
        .into_iter()
        .partition(|(_, path)| crate::app::escapes_root(root, path));
    let outside = outside
        .into_iter()
        // `escapes_root` is false for a path that will not canonicalize, so
        // every one here did a moment ago; one that vanished since keeps
        // its spelling and fails to open like any missing file.
        .map(|(b, p)| (b, p.canonicalize().unwrap_or(p)))
        .collect();
    (inside, outside)
}

/// An image block and the file it names.
pub type Request = (BlockIdx, PathBuf);

/// Images in document order, including unavailable images whose alt text is useful.
#[must_use]
pub fn image_blocks(doc: &Document) -> Vec<BlockIdx> {
    (0..doc.block_count())
        .map(|b| BlockIdx(b as u32))
        .filter(|b| matches!(doc.node_for_block(*b).kind, NodeKind::Image { .. }))
        .collect()
}

/// Open `path` for decoding only if it is a regular file.
///
/// A FIFO reports length zero, so the size cap passed it, and open(2) on a
/// FIFO with no writer blocks forever: `![pipe](p.png)` stalled the worker,
/// every image after it never arrived, and each live reload stranded one
/// more thread in open(2) — thirteen after twelve reloads. The same trap
/// `app::check_document_size` closes for documents, closed by the one
/// helper config and state files are read through.
fn open_regular(path: &Path) -> Result<(std::fs::File, u64), String> {
    carrel_state::state::open_regular(path).map_err(|e| e.to_string())
}

/// Decode one file within the caps. Blocking — worker-thread only.
fn decode_one(path: &Path) -> Result<image::DynamicImage, String> {
    let (file, len) = open_regular(path)?;
    if len > MAX_IMAGE_FILE_BYTES {
        return Err(format!(
            "{len} bytes is past the {MAX_IMAGE_FILE_BYTES}-byte cap"
        ));
    }
    let mut reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_AXIS_PX);
    limits.max_image_height = Some(MAX_IMAGE_AXIS_PX);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

/// Shrink an image wider than `max_w` to that width, aspect kept.
///
/// `thumbnail` rather than a filtered resize: it is a box filter over the
/// source, about a tenth the cost on an 8000-pixel image, and the result is
/// headed for half-block cells or a terminal's own scaler anyway.
fn fit_width(img: image::DynamicImage, max_w: u32) -> image::DynamicImage {
    if img.width() <= max_w.max(1) {
        return img;
    }
    img.thumbnail(max_w.max(1), u32::MAX)
}

/// Decode every request on a worker thread. Returns immediately.
///
/// The receiver yields one message per request, in completion order. Dropping
/// it stops the worker at its next send — the same lifecycle as the scan.
///
/// Each file is decoded once, keyed by its canonical path, however many
/// blocks name it; the cache lives as long as this worker, which is one
/// document's open, so a file edited later is decoded afresh next time.
#[must_use]
pub fn spawn_decoder(reqs: Vec<(BlockIdx, PathBuf)>, caps: DecodeCaps) -> Receiver<ImageMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        decode_all(reqs, caps, decode_one, |msg| tx.send(msg).is_ok());
    });
    rx
}

/// The worker's loop, with the decoder and the channel passed in so a test
/// can count decodes. `send` returning `false` stops it.
fn decode_all(
    reqs: Vec<(BlockIdx, PathBuf)>,
    caps: DecodeCaps,
    mut decode: impl FnMut(&Path) -> Result<image::DynamicImage, String>,
    mut send: impl FnMut(ImageMsg) -> bool,
) {
    let mut decoded: HashMap<PathBuf, Result<image::DynamicImage, String>> = HashMap::new();
    let mut spent: u64 = 0;
    for (block, path) in reqs {
        let key = path.canonicalize().unwrap_or_else(|_| path.clone());
        let result = decoded
            .entry(key)
            .or_insert_with(|| decode(&path).map(|img| fit_width(img, caps.max_width_px)));
        let msg = match result {
            Ok(img) => {
                let bytes = img.as_bytes().len() as u64;
                if spent.saturating_add(bytes) > caps.budget_bytes {
                    ImageMsg::Failed(
                        block,
                        format!(
                            "{}: past the {}-byte image budget for one document",
                            path.display(),
                            caps.budget_bytes
                        ),
                    )
                } else {
                    spent += bytes;
                    ImageMsg::Decoded(block, img.clone())
                }
            }
            Err(e) => ImageMsg::Failed(block, format!("{}: {e}", path.display())),
        };
        if !send(msg) {
            return;
        }
    }
}

/// Rows an image occupies when fitted to `avail_cols`, at the terminal's font
/// pixel size. Pure arithmetic — this is the number the layout override uses.
#[must_use]
pub fn rows_for_dims(px: (u32, u32), font_px: (u16, u16), avail_cols: u16) -> u32 {
    let (img_w, img_h) = (f64::from(px.0.max(1)), f64::from(px.1.max(1)));
    let (font_w, font_h) = (f64::from(font_px.0.max(1)), f64::from(font_px.1.max(1)));
    let avail_px = f64::from(avail_cols.max(1)) * font_w;
    // Fit to width, never upscale past natural size.
    let scale = (avail_px / img_w).min(1.0);
    let rows = (img_h * scale / font_h).ceil();
    #[allow(clippy::cast_sign_loss)]
    let rows = rows.max(1.0) as u32;
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use carrel_core::Document;

    fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let p = dir.join(name);
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200u8, 40, 40]));
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn requests_cover_local_images_and_skip_remote_ones() {
        let doc = Document::parse(
            "![local](pic.png)\n\n![web](https://example.com/x.png)\n\n![data](data:image/png;base64,xxxx)\n",
        );
        let reqs = local_image_requests(&doc, Some(Path::new("/docs")));
        assert_eq!(reqs.len(), 1, "{reqs:?}");
        assert_eq!(reqs[0].1, Path::new("/docs/pic.png"));
    }

    #[test]
    fn no_base_directory_means_no_relative_requests() {
        let doc = Document::parse("![local](pic.png)\n");
        assert_eq!(
            local_image_requests(&doc, None),
            [] as [(carrel_core::BlockIdx, std::path::PathBuf); 0]
        );
    }

    #[test]
    fn the_worker_decodes_a_real_png_and_reports_a_missing_one() {
        let d = tempfile::tempdir().unwrap();
        let ok = write_png(d.path(), "ok.png", 12, 7);
        let reqs = vec![
            (BlockIdx(0), ok),
            (BlockIdx(1), d.path().join("missing.png")),
        ];
        let mut decoded = 0;
        let mut failed = 0;
        for msg in spawn_decoder(reqs, DecodeCaps::default()) {
            match msg {
                ImageMsg::Decoded(b, img) => {
                    assert_eq!(b, BlockIdx(0));
                    assert_eq!((img.width(), img.height()), (12, 7));
                    decoded += 1;
                }
                ImageMsg::Failed(b, e) => {
                    assert_eq!(b, BlockIdx(1));
                    assert!(e.contains("missing.png"), "{e}");
                    failed += 1;
                }
            }
        }
        assert_eq!((decoded, failed), (1, 1));
    }

    #[test]
    fn a_file_that_is_not_an_image_fails_cleanly() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("fake.png"), b"not a png at all").unwrap();
        let msgs: Vec<_> = spawn_decoder(
            vec![(BlockIdx(0), d.path().join("fake.png"))],
            DecodeCaps::default(),
        )
        .iter()
        .collect();
        assert!(matches!(msgs[0], ImageMsg::Failed(..)));
    }

    #[test]
    fn row_arithmetic_fits_to_width_and_never_upscales() {
        // 100×50 px at font (10, 20): natural width is 10 cols. At 20 cols
        // avail it must NOT upscale: 50 px tall / 20 px per row = 2.5 → 3 rows.
        assert_eq!(rows_for_dims((100, 50), (10, 20), 20), 3);
        // At 5 cols avail it scales down by half: 25 px / 20 → 2 rows.
        assert_eq!(rows_for_dims((100, 50), (10, 20), 5), 2);
        // Degenerate dims clamp to one row.
        assert_eq!(rows_for_dims((0, 0), (10, 20), 20), 1);
    }

    fn collect(
        reqs: Vec<(BlockIdx, PathBuf)>,
        caps: DecodeCaps,
        decodes: &mut usize,
    ) -> Vec<ImageMsg> {
        let mut out = Vec::new();
        decode_all(
            reqs,
            caps,
            |p| {
                *decodes += 1;
                decode_one(p)
            },
            |m| {
                out.push(m);
                true
            },
        );
        out
    }

    #[test]
    fn many_references_to_one_file_decode_it_once() {
        let d = tempfile::tempdir().unwrap();
        let pic = write_png(d.path(), "pic.png", 10, 10);
        // A second spelling of the same file shares the decode too.
        let other = d.path().join(".").join("pic.png");
        let reqs = (0..5)
            .map(|b| {
                let path = if b % 2 == 0 {
                    pic.clone()
                } else {
                    other.clone()
                };
                (BlockIdx(b), path)
            })
            .collect();
        let mut decodes = 0;
        let msgs = collect(reqs, DecodeCaps::default(), &mut decodes);
        assert_eq!(decodes, 1, "one file, one decode");
        assert_eq!(msgs.len(), 5);
        assert!(msgs.iter().all(|m| matches!(m, ImageMsg::Decoded(..))));
    }

    #[test]
    fn the_document_budget_degrades_the_overflow_to_alt_text() {
        let d = tempfile::tempdir().unwrap();
        // 100×100 RGB is 30 000 bytes: room for two blocks, not three.
        let pic = write_png(d.path(), "pic.png", 100, 100);
        let reqs = (0..3).map(|b| (BlockIdx(b), pic.clone())).collect();
        let caps = DecodeCaps {
            budget_bytes: 70_000,
            ..DecodeCaps::default()
        };
        let msgs = collect(reqs, caps, &mut 0);
        assert!(matches!(msgs[0], ImageMsg::Decoded(BlockIdx(0), _)));
        assert!(matches!(msgs[1], ImageMsg::Decoded(BlockIdx(1), _)));
        let ImageMsg::Failed(BlockIdx(2), e) = &msgs[2] else {
            panic!("{msgs:?}");
        };
        assert!(e.contains("budget"), "{e}");
    }

    #[test]
    fn a_wide_image_is_shrunk_to_the_ceiling_at_decode() {
        let d = tempfile::tempdir().unwrap();
        let pic = write_png(d.path(), "wide.png", 300, 100);
        let caps = DecodeCaps {
            max_width_px: 60,
            ..DecodeCaps::default()
        };
        let msgs = collect(vec![(BlockIdx(0), pic)], caps, &mut 0);
        let ImageMsg::Decoded(_, img) = &msgs[0] else {
            panic!("{msgs:?}");
        };
        assert_eq!((img.width(), img.height()), (60, 20));
    }

    #[test]
    fn images_outside_the_root_are_refused_symlinks_included() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        let out = d.path().join("outside");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let inside = write_png(&lib, "in.png", 2, 2);
        let secret = write_png(&out, "secret.png", 2, 2);
        std::os::unix::fs::symlink(&secret, lib.join("link.png")).unwrap();
        let doc = Document::parse(&format!(
            "![a](in.png)\n\n![b](../outside/secret.png)\n\n![c]({})\n\n![d](link.png)\n",
            secret.display()
        ));
        let reqs = local_image_requests(&doc, Some(&lib));
        assert_eq!(reqs.len(), 4, "{reqs:?}");
        let (kept, refused) = contain(reqs, Some(&lib));
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(kept[0].1, inside);
        let refused: Vec<_> = refused.into_iter().map(|(b, _)| b).collect();
        assert_eq!(refused, [BlockIdx(1), BlockIdx(2), BlockIdx(3)]);
        // No root at all (an App built by hand) refuses nothing.
        let (kept, refused) = contain(local_image_requests(&doc, Some(&lib)), None);
        assert_eq!((kept.len(), refused.len()), (4, 0));
    }

    #[test]
    fn a_fifo_is_refused_without_blocking_and_the_next_image_still_arrives() {
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("p.png");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            return; // no mkfifo here; nothing to prove
        }
        let red = write_png(d.path(), "red.png", 3, 3);
        // On a worker, with a deadline: the bug is a hang, and a hang must
        // fail the test rather than wedge the suite.
        let rx = spawn_decoder(
            vec![(BlockIdx(0), fifo), (BlockIdx(1), red)],
            DecodeCaps::default(),
        );
        let wait = std::time::Duration::from_secs(5);
        let first = rx.recv_timeout(wait).expect("the FIFO blocked the decoder");
        let ImageMsg::Failed(BlockIdx(0), e) = first else {
            panic!("{first:?}");
        };
        assert!(e.contains("not a regular file"), "{e}");
        let second = rx.recv_timeout(wait).expect("the next image never came");
        assert!(
            matches!(second, ImageMsg::Decoded(BlockIdx(1), _)),
            "{second:?}"
        );
    }

    /// proj/docs/x.md with `../assets/y.png`, `local.png` and an absolute
    /// path to y.png, read from a file list rooted at `root`.
    fn reading_x_from(root: &str) -> (tempfile::TempDir, crate::app::App) {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().canonicalize().unwrap().join("proj");
        std::fs::create_dir_all(proj.join("docs")).unwrap();
        std::fs::create_dir_all(proj.join("assets")).unwrap();
        let y = write_png(&proj.join("assets"), "y.png", 2, 2);
        write_png(&proj.join("docs"), "local.png", 2, 2);
        let x = proj.join("docs/x.md");
        std::fs::write(
            &x,
            format!(
                "![up](../assets/y.png)\n\n![local](local.png)\n\n![abs]({})\n",
                y.display()
            ),
        )
        .unwrap();
        let mut a = crate::app::App::new_home(proj.join(root), vec![], 80, 24);
        a.open_path(&x).unwrap();
        (d, a)
    }

    fn refused(a: &crate::app::App) -> usize {
        a.image_errors
            .values()
            .filter(|e| *e == OUTSIDE_FOLDER)
            .count()
    }

    #[test]
    fn moving_the_file_list_up_replans_the_same_document() {
        use crate::action::Action;
        let (_d, mut a) = reading_x_from("docs");
        let before = sync_key(&a);
        assert_eq!(plan(&mut a).len(), 1);
        assert_eq!(refused(&a), 2);
        assert_eq!(a.note.as_deref(), Some(outside_note(2).as_str()));
        // q, Backspace, Enter on the same file.
        let x = a.file.clone().unwrap();
        crate::app::update(&mut a, Action::CloseFile);
        crate::app::update(&mut a, Action::HomeUp);
        a.open_path(&x).unwrap();
        assert_ne!(sync_key(&a), before, "a new root must replan");
        assert_eq!(plan(&mut a).len(), 3);
        assert_eq!(refused(&a), 0);
    }

    #[test]
    fn narrowing_the_root_refuses_what_it_used_to_allow() {
        let (_d, mut a) = reading_x_from("");
        let before = sync_key(&a);
        assert_eq!(plan(&mut a).len(), 3);
        a.image_dims.insert(BlockIdx(0), (2, 2));
        // The same parse planned under a narrower root forgets what the
        // wider one decoded.
        let docs = a.file.as_deref().unwrap().parent().unwrap().to_path_buf();
        a.home_stash.as_mut().unwrap().root = docs;
        assert_ne!(sync_key(&a), before);
        assert_eq!(plan(&mut a).len(), 1);
        assert_eq!(refused(&a), 2);
        assert!(a.image_dims.is_empty(), "{:?}", a.image_dims);
        // And widening again forgets the refusals the narrow root made.
        let proj = a
            .home_stash
            .as_ref()
            .unwrap()
            .root
            .parent()
            .unwrap()
            .to_path_buf();
        a.home_stash.as_mut().unwrap().root = proj;
        assert_eq!(plan(&mut a).len(), 3);
        assert_eq!(refused(&a), 0);
    }

    #[test]
    fn the_question_names_where_an_image_leads_by_its_shortest_spelling() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        let target = base.join("outside/red.png");
        let root = base.join("docs");
        assert_eq!(
            outside_where("link.png", &target, Some(&root)),
            "link.png \u{2192} ../outside/red.png"
        );
        assert_eq!(
            outside_where("../outside/red.png", &target, Some(&root)),
            "../outside/red.png"
        );
        // Far away, the absolute path is the shorter one.
        let far = Path::new("/x.png");
        assert_eq!(outside_where("/x.png", far, Some(&root)), "/x.png");
        // A control in a name is escaped, never painted.
        assert!(!outside_where("a\x1b.png", &target, Some(&root)).contains('\x1b'));
    }

    #[test]
    fn a_long_path_keeps_its_end_and_wraps_whole_when_it_can() {
        let p = "/tmp/claude-1000/very/long/scratchpad/outside/red.png";
        assert_eq!(keep_end(p, 12), "\u{2026}ide/red.png");
        assert_eq!(carrel_core::display_width(&keep_end(p, 12)), 12);
        let rows = wrap_cells(p, 20);
        assert_eq!(rows.concat(), p, "nothing lost");
        assert!(rows.iter().all(|r| carrel_core::display_width(r) <= 20));
        // Room for all of it: whole. Room for little: the end, then the ask.
        let q = outside_question(p, 20, 9);
        assert!(q.concat().contains("red.png") && q.concat().replace(' ', "").contains("[show]"));
        let q = outside_question(p, 20, 2);
        assert_eq!(q.len(), 2);
        assert!(q[0].ends_with("red.png"), "{q:?}");
        let q = outside_question(p, 20, 1);
        assert_eq!(q, vec![keep_end(p, 20)]);
    }

    #[test]
    fn the_note_agrees_with_its_count() {
        assert_eq!(
            outside_note(1),
            "1 image outside this folder, so it was not opened"
        );
        assert_eq!(
            outside_note(2),
            "2 images outside this folder, so they were not opened"
        );
    }
}
