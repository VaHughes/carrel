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
//! An image outside the folder carrel was pointed at is never opened at all
//! ([`contain`]) — the same rule, through the same function, that makes a
//! link out of the folder ask first.

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
/// second Enter; an image has no Enter to wait for, so it is simply not
/// opened and keeps its alt text. `root` is `App::library_root`, so the rule
/// is the links' rule in every case: a file opened directly is rooted at its
/// own folder, a pipe at the working directory, the home list at its root.
#[must_use]
pub fn contain(
    reqs: Vec<(BlockIdx, PathBuf)>,
    root: Option<&Path>,
) -> (Vec<(BlockIdx, PathBuf)>, Vec<BlockIdx>) {
    let Some(root) = root else {
        return (reqs, Vec::new());
    };
    let (outside, inside): (Vec<_>, Vec<_>) = reqs
        .into_iter()
        .partition(|(_, path)| crate::app::escapes_root(root, path));
    (inside, outside.into_iter().map(|(b, _)| b).collect())
}

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
/// `app::check_document_size` closes for documents. The open is
/// non-blocking and the check is on the OPENED handle, so a regular file
/// swapped for a FIFO between a stat and the open cannot block either; a
/// regular file ignores `O_NONBLOCK`.
fn open_regular(path: &Path) -> Result<(std::fs::File, u64), String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let file = opts.open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file, so it was not read".into());
    }
    Ok((file, meta.len()))
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
}
