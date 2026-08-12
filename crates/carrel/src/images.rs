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

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use carrel_core::{BlockIdx, Document, NodeKind};

/// Files larger than this are not decoded. A 64 MiB "figure" is a mistake.
pub const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Per-axis pixel cap fed to [`image::Limits`].
pub const MAX_IMAGE_AXIS_PX: u32 = 8192;
/// Decoder allocation cap fed to [`image::Limits`].
pub const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

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

/// Decode one file within the caps. Blocking — worker-thread only.
fn decode_one(path: &Path) -> Result<image::DynamicImage, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_IMAGE_FILE_BYTES {
        return Err(format!(
            "{len} bytes is past the {MAX_IMAGE_FILE_BYTES}-byte cap"
        ));
    }
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_AXIS_PX);
    limits.max_image_height = Some(MAX_IMAGE_AXIS_PX);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

/// Decode every request on a worker thread. Returns immediately.
///
/// The receiver yields one message per request, in completion order. Dropping
/// it stops the worker at its next send — the same lifecycle as the scan.
#[must_use]
pub fn spawn_decoder(reqs: Vec<(BlockIdx, PathBuf)>) -> Receiver<ImageMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (block, path) in reqs {
            let msg = match decode_one(&path) {
                Ok(img) => ImageMsg::Decoded(block, img),
                Err(e) => ImageMsg::Failed(block, format!("{}: {e}", path.display())),
            };
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
    rx
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
        assert!(local_image_requests(&doc, None).is_empty());
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
        for msg in spawn_decoder(reqs) {
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
        let msgs: Vec<_> = spawn_decoder(vec![(BlockIdx(0), d.path().join("fake.png"))])
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
}
