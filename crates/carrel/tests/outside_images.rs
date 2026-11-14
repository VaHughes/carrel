//! An image outside the folder is refused until the reader, looking at its
//! path in the lightbox, asks for it; then it shows for this session: that
//! one file, nowhere else, never on disk.
use carrel::action::Action;
use carrel::app::{App, Outcome, update};
use carrel::images::{self, ImageMsg, OUTSIDE_FOLDER};
use carrel_core::BlockIdx;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::{Path, PathBuf};

fn png(path: &Path) {
    image::RgbImage::from_pixel(4, 2, image::Rgb([9u8, 9, 9]))
        .save(path)
        .unwrap();
}

/// `lib/x.md` naming two images in `out/` and one beside it, read with
/// `lib` as the root.
fn fixture() -> (tempfile::TempDir, App, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let base = d.path().canonicalize().unwrap();
    std::fs::create_dir_all(base.join("lib")).unwrap();
    std::fs::create_dir_all(base.join("out")).unwrap();
    png(&base.join("out/y.png"));
    png(&base.join("out/z.png"));
    png(&base.join("lib/in.png"));
    let x = base.join("lib/x.md");
    std::fs::write(
        &x,
        "![why](../out/y.png)\n\n![zed](../out/z.png)\n\n![inn](in.png)\n",
    )
    .unwrap();
    let mut a = App::new_home(base.join("lib"), vec![], 80, 24);
    a.open_path(&x).unwrap();
    (d, a, base)
}

fn refused(a: &App) -> Vec<BlockIdx> {
    let mut v: Vec<_> = a.image_outside.keys().copied().collect();
    v.sort();
    v
}

fn block_naming(a: &App, file: &str) -> BlockIdx {
    *a.image_outside
        .iter()
        .find(|(_, p)| p.ends_with(file))
        .unwrap()
        .0
}

/// What the reader does: open the lightbox over the image, which names
/// its path, and answer it.
fn allow(a: &mut App, block: BlockIdx) -> Outcome {
    update(a, Action::ImageOpen(Some(block)));
    let plan = a.image_plan;
    update(a, Action::ImageAllow(block, plan))
}

#[test]
fn the_lightbox_answer_shows_that_image_and_no_other() {
    let (_d, mut a, base) = fixture();
    assert_eq!(images::plan(&mut a).len(), 1, "only the one inside");
    assert_eq!(refused(&a).len(), 2);
    let y = block_naming(&a, "y.png");
    let z = block_naming(&a, "z.png");
    let before = images::sync_key(&a);

    assert_eq!(allow(&mut a, y), Outcome::Redraw);
    assert_ne!(images::sync_key(&a), before, "consent must replan");
    let reqs = images::plan(&mut a);
    assert!(reqs.contains(&(y, base.join("out/y.png"))), "{reqs:?}");
    assert_eq!(reqs.len(), 2);
    assert_eq!(refused(&a), vec![z], "the other image is still refused");
    assert_eq!(
        a.image_errors.get(&z).map(String::as_str),
        Some(OUTSIDE_FOLDER)
    );
    assert!(!a.image_errors.contains_key(&y));

    // A reload of the same file keeps the consent.
    a.reload().unwrap();
    assert_eq!(images::plan(&mut a).len(), 2);
    assert_eq!(refused(&a).len(), 1);

    // A restart — a new App — asks again: nothing was written anywhere.
    let x = a.file.clone().unwrap();
    let mut fresh = App::new_home(base.join("lib"), vec![], 80, 24);
    fresh.open_path(&x).unwrap();
    assert_eq!(images::plan(&mut fresh).len(), 1);
    assert_eq!(refused(&fresh).len(), 2);
}

/// Consent re-plans the consented block alone: the picture already decoded
/// keeps its dimensions, so nothing collapses to alt text and back.
#[test]
fn consent_plans_only_the_consented_block() {
    let (_d, mut a, base) = fixture();
    let first = images::plan(&mut a);
    let inn = first[0].0;
    a.image_dims.insert(inn, (4, 2));
    let y = block_naming(&a, "y.png");
    let z = block_naming(&a, "z.png");
    allow(&mut a, y);
    let reqs = images::plan_consented(&mut a);
    assert_eq!(reqs, vec![(y, base.join("out/y.png"))]);
    assert_eq!(a.image_dims.get(&inn), Some(&(4, 2)), "kept");
    assert_eq!(refused(&a), vec![z]);
    assert!(!a.image_errors.contains_key(&y));
    let msgs: Vec<_> = images::spawn_decoder(reqs, images::DecodeCaps::default())
        .iter()
        .collect();
    assert!(
        matches!(msgs[..], [ImageMsg::Decoded(b, _)] if b == y),
        "{msgs:?}"
    );
}

#[test]
fn consent_is_to_the_file_a_symlink_named_not_to_the_symlink() {
    let (_d, mut a, base) = fixture();
    let link = base.join("out/link.png");
    std::os::unix::fs::symlink(base.join("out/y.png"), &link).unwrap();
    std::fs::write(a.file.as_ref().unwrap(), "![l](../out/link.png)\n").unwrap();
    a.reload().unwrap();
    images::plan(&mut a);
    let b = refused(&a)[0];
    assert_eq!(a.image_outside[&b], base.join("out/y.png"), "canonical");
    allow(&mut a, b);
    assert_eq!(images::plan(&mut a).len(), 1);
    // The link swung to another file is another file, and is refused.
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(base.join("out/z.png"), &link).unwrap();
    assert!(images::plan(&mut a).is_empty());
    assert_eq!(refused(&a).len(), 1);
}

#[test]
fn an_allow_with_no_question_on_screen_does_nothing() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let y = block_naming(&a, "y.png");
    let z = block_naming(&a, "z.png");
    let plan = a.image_plan;
    // No lightbox: a press on the block, or a hand-built action.
    assert_eq!(update(&mut a, Action::ImageAllow(y, plan)), Outcome::Idle);
    // The lightbox up over ANOTHER image.
    update(&mut a, Action::ImageOpen(Some(z)));
    assert_eq!(update(&mut a, Action::ImageAllow(y, plan)), Outcome::Idle);
    // A block that was never refused.
    assert_eq!(
        update(&mut a, Action::ImageAllow(BlockIdx(99), plan)),
        Outcome::Idle
    );
    assert!(a.image_consent.is_empty());
}

/// A target painted for one parse, delivered after a reload, must not
/// consent to whatever refused image now has that block index: a file the
/// reader was never shown (cold review, 2026-11-14).
#[test]
fn a_stale_allow_consents_to_nothing_after_a_reload() {
    let (_d, mut a, base) = fixture();
    let x = a.file.clone().unwrap();
    std::fs::write(&x, "![a cute cat](../out/y.png)\n").unwrap();
    a.reload().unwrap();
    images::plan(&mut a);
    let shown = refused(&a)[0];
    update(&mut a, Action::ImageOpen(Some(shown)));
    let stale = Action::ImageAllow(shown, a.image_plan);
    // The agent rewrites the file; the loop reloads and replans, and the
    // reader reopens the lightbox, before the queued click arrives.
    std::fs::write(&x, "![a cute cat](../out/z.png)\n").unwrap();
    a.reload().unwrap();
    images::plan(&mut a);
    update(&mut a, Action::ImageOpen(Some(shown)));
    assert_eq!(update(&mut a, stale), Outcome::Idle);
    assert!(
        !a.image_consent.contains(&base.join("out/z.png")),
        "{:?}",
        a.image_consent
    );
}

fn frame(a: &App) -> (String, carrel::render::Painted) {
    let mut t = Terminal::new(TestBackend::new(a.cols, a.rows)).unwrap();
    let mut painted = carrel::render::Painted::default();
    t.draw(|f| {
        carrel::render::draw_full(f, a, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let buf = t.backend().buffer();
    let text = (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, painted)
}

/// The cell where `needle` is painted.
fn cell_of(text: &str, needle: &str) -> (u16, u16) {
    for (y, line) in text.lines().enumerate() {
        if let Some(i) = line.find(needle) {
            let col = line[..i].chars().count();
            return (u16::try_from(col).unwrap(), u16::try_from(y).unwrap());
        }
    }
    panic!("{needle:?} not on screen:\n{text}");
}

/// The alt text is the document's to choose — it can read like several
/// rows of prose — and a target fires on the press, so a press meant to
/// start a selection consented before any file name was on screen. The
/// block and its chip open the lightbox, which names the path.
#[test]
fn a_press_on_a_refused_block_asks_and_never_consents() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let (text, painted) = frame(&a);
    assert!(text.contains(images::SHOW_OUTSIDE), "{text}");
    let z = block_naming(&a, "z.png");
    let (x, y) = cell_of(&text, "zed");
    let (cx, cy) = cell_of(&text, "zed [outside this folder");
    for (x, y) in [(x, y), (cx + 6, cy)] {
        let action = painted.targets.hit(x, y).unwrap().action;
        assert_eq!(action, Action::ImageOpen(Some(z)));
        update(&mut a, action);
    }
    assert!(a.image_consent.is_empty(), "a press consents to nothing");
    assert_eq!(a.lightbox, Some(z));
    let (text, _) = frame(&a);
    assert!(text.contains("z.png is outside this folder"), "{text}");
}

#[test]
fn the_context_menu_on_a_refused_image_opens_the_question() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let z = block_naming(&a, "z.png");
    let byte = a.doc.node_for_block(z).doc.start;
    let items = carrel::menu::context(&a, byte);
    let row = items
        .iter()
        .find(|i| i.action == Some(Action::ImageOpen(Some(z))))
        .expect("a row that opens the lightbox");
    assert!(row.enabled);
    assert!(
        items
            .iter()
            .all(|i| !matches!(i.action, Some(Action::ImageAllow(..)))),
        "no row consents before the path is shown"
    );
    // The control: on an image that is not refused, no such row.
    allow(&mut a, z);
    images::plan(&mut a);
    assert!(
        carrel::menu::context(&a, byte)
            .iter()
            .all(|i| i.action != Some(Action::ImageOpen(Some(z))))
    );
}

#[test]
fn the_lightbox_over_a_refused_image_asks_and_its_show_button_answers() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let y = block_naming(&a, "y.png");
    update(&mut a, Action::ImageOpen(Some(y)));
    let (text, painted) = frame(&a);
    assert!(text.contains("Enter or [show] to show it"), "{text}");
    let (x, row) = cell_of(&text, "[show] [close]");
    let hit = painted.targets.hit(x, row).unwrap();
    assert_eq!(hit.action, Action::ImageAllow(y, a.image_plan));
    assert_eq!(update(&mut a, hit.action), Outcome::Redraw);
    assert_eq!(a.lightbox, Some(y), "the lightbox stays to show it");
    images::plan(&mut a);
    let (text, _) = frame(&a);
    assert!(text.contains("Loading image"), "{text}");
    assert!(!text.contains("[show]"), "{text}");
}
