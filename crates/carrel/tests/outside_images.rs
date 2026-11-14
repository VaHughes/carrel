//! An image outside the folder is refused until the reader clicks it, and
//! then shows for this session: that one file, nowhere else, never on disk.
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

/// `lib/x.md` naming two images in `out/`, read with `lib` as the root.
fn fixture() -> (tempfile::TempDir, App, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let base = d.path().canonicalize().unwrap();
    std::fs::create_dir_all(base.join("lib")).unwrap();
    std::fs::create_dir_all(base.join("out")).unwrap();
    png(&base.join("out/y.png"));
    png(&base.join("out/z.png"));
    let x = base.join("lib/x.md");
    std::fs::write(&x, "![why](../out/y.png)\n\n![zed](../out/z.png)\n").unwrap();
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

#[test]
fn a_click_shows_that_image_and_no_other() {
    let (_d, mut a, base) = fixture();
    assert!(images::plan(&mut a).is_empty());
    assert_eq!(refused(&a).len(), 2);
    let y = block_naming(&a, "y.png");
    let z = block_naming(&a, "z.png");
    let before = images::sync_key(&a);

    assert_eq!(update(&mut a, Action::ImageAllow(y)), Outcome::Redraw);
    assert_ne!(images::sync_key(&a), before, "consent must replan");
    let reqs = images::plan(&mut a);
    assert_eq!(reqs, vec![(y, base.join("out/y.png"))]);
    assert_eq!(refused(&a), vec![z], "the other image is still refused");
    assert_eq!(
        a.image_errors.get(&z).map(String::as_str),
        Some(OUTSIDE_FOLDER)
    );
    assert!(!a.image_errors.contains_key(&y));
    // And the worker decodes it, under the same guards as any image.
    let msgs: Vec<_> = images::spawn_decoder(reqs, images::DecodeCaps::default())
        .iter()
        .collect();
    assert!(
        matches!(msgs[..], [ImageMsg::Decoded(b, _)] if b == y),
        "{msgs:?}"
    );

    // A reload of the same file keeps the consent.
    a.reload().unwrap();
    assert_eq!(images::plan(&mut a).len(), 1);
    assert_eq!(refused(&a).len(), 1);

    // A restart — a new App — asks again: nothing was written anywhere.
    let x = a.file.clone().unwrap();
    let mut fresh = App::new_home(base.join("lib"), vec![], 80, 24);
    fresh.open_path(&x).unwrap();
    assert!(images::plan(&mut fresh).is_empty());
    assert_eq!(refused(&fresh).len(), 2);
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
    update(&mut a, Action::ImageAllow(b));
    assert_eq!(images::plan(&mut a).len(), 1);
    // The link swung to another file is another file, and is refused.
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(base.join("out/z.png"), &link).unwrap();
    assert!(images::plan(&mut a).is_empty());
    assert_eq!(refused(&a).len(), 1);
}

#[test]
fn an_allow_for_a_block_that_was_not_refused_does_nothing() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let consent = a.image_consent.len();
    assert_eq!(
        update(&mut a, Action::ImageAllow(BlockIdx(99))),
        Outcome::Idle
    );
    assert_eq!(a.image_consent.len(), consent);
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

#[test]
fn a_refused_block_is_a_button_that_says_so_and_a_shown_one_is_not() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let (text, painted) = frame(&a);
    assert!(text.contains(images::SHOW_OUTSIDE), "{text}");
    let (x, y) = cell_of(&text, "zed");
    let z = block_naming(&a, "z.png");
    assert_eq!(
        painted.targets.hit(x, y).unwrap().action,
        Action::ImageAllow(z)
    );
    // The chip trails the alt text, and is part of the button.
    let (cx, cy) = cell_of(&text, "zed [outside this folder");
    let cx = cx + 6;
    assert_eq!(
        painted.targets.hit(cx, cy).unwrap().action,
        Action::ImageAllow(z)
    );
    // Once allowed (and still loading), the block opens the lightbox again
    // and wears no chip.
    update(&mut a, Action::ImageAllow(z));
    images::plan(&mut a);
    let (text, painted) = frame(&a);
    let (x, y) = cell_of(&text, "zed");
    assert_eq!(
        painted.targets.hit(x, y).unwrap().action,
        Action::ImageOpen(Some(z))
    );
    assert_eq!(text.matches("click to show").count(), 1, "{text}");
}

#[test]
fn the_context_menu_on_a_refused_image_offers_to_show_it() {
    let (_d, mut a, _) = fixture();
    images::plan(&mut a);
    let z = block_naming(&a, "z.png");
    let byte = a.doc.node_for_block(z).doc.start;
    let items = carrel::menu::context(&a, byte);
    let row = items
        .iter()
        .find(|i| i.action == Some(Action::ImageAllow(z)))
        .expect("a Show row");
    assert!(row.enabled);
    assert_eq!(
        row.accel, "",
        "no key printed: Enter here opens the lightbox"
    );
    // The control: on an image that is not refused, no such row.
    update(&mut a, Action::ImageAllow(z));
    images::plan(&mut a);
    assert!(
        carrel::menu::context(&a, byte)
            .iter()
            .all(|i| !matches!(i.action, Some(Action::ImageAllow(_))))
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
    assert_eq!(hit.action, Action::ImageAllow(y));
    assert_eq!(update(&mut a, hit.action), Outcome::Redraw);
    assert_eq!(a.lightbox, Some(y), "the lightbox stays to show it");
    images::plan(&mut a);
    let (text, _) = frame(&a);
    assert!(text.contains("Loading image"), "{text}");
    assert!(!text.contains("[show]"), "{text}");
}
