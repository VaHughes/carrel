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

/// A kept place survives an edit made while carrel was closed: with a
/// paragraph inserted at the top, EVERY display byte of the old document —
/// in headings, lists, code, quotes, tables, the separators a gap row is
/// named by — resolves to the same character, moved down by the insertion.
/// A plain source place lands that many bytes early, in other text.
#[test]
fn a_kept_place_follows_its_text_down_past_an_insertion() {
    let old = "# Title\n\nA paragraph with \"quotes\" -- and more.\n\n- item one\n- item two\n\n\
> quoted words\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nLast line.\n";
    let new = format!("Inserted while carrel was closed.\n\n{old}");
    let (a, b) = (
        carrel_core::Document::parse(old),
        carrel_core::Document::parse(&new),
    );
    let shift = u32::try_from(b.text.len() - a.text.len()).unwrap();
    assert!(
        b.text.ends_with(&*a.text),
        "the fixture shifts the text whole"
    );
    let mut moved = 0;
    for (at, _) in a.text.char_indices() {
        let d = carrel_core::DocByte(at as u32);
        let kept = Place::kept(&a, d);
        assert_eq!(
            kept.resolve(&b).0,
            d.0 + shift,
            "{kept:?} at {:?}",
            &a.text[at..a.text.ceil_char_boundary(at + 12)]
        );
        // Unchanged, it is still exact.
        assert_eq!(kept.resolve(&a), d);
        if Place::of(&a, d).resolve(&b).0 != d.0 + shift {
            moved += 1;
        }
    }
    assert!(moved > 0, "the control: a bare source place does move");
}
