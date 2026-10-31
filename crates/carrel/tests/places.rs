//! Saved places (`carrel_state::state::Place`) against real documents.
//!
//! Here rather than beside `Place` because the corpus is this crate's, and a
//! test in `carrel-state` reaching into another crate's directory would
//! fail for anyone running the published crate's tests.

use carrel_state::state::Place;

/// Under the core that saved it, a place gives back the display byte it
/// was taken from — EVERY one of them, not only the row starts a reader
/// usually saves: inside smart quotes, an autolink, a table's padding,
/// an alert's label, a math block. A source byte alone fails here,
/// which is what the correction is for: provenance is per run, and a
/// rewritten run maps all of its display bytes to its first source byte.
#[test]
fn under_the_core_that_saved_it_every_place_resolves_exactly() {
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");
    let mut checked = 0;
    for f in std::fs::read_dir(corpus).unwrap() {
        let path = f.unwrap().path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let doc = carrel_core::Document::parse(&std::fs::read_to_string(&path).unwrap());
        for (at, _) in doc.text.char_indices() {
            let d = carrel_core::DocByte(at as u32);
            let place = Place::of(&doc, d);
            assert_eq!(
                place.resolve(&doc),
                d,
                "{}: {place:?} near {:?}",
                path.display(),
                &doc.text[at..doc.text.ceil_char_boundary(at + 16)],
            );
            checked += 1;
        }
    }
    assert!(checked > 1000, "the corpus went missing: {checked}");
}
