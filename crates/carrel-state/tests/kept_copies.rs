use carrel_core::{DocByte, Document};
use carrel_state::state::Place;

fn at(doc: &Document, needle: &str, nth: usize) -> DocByte {
    let i = doc.text.match_indices(needle).nth(nth).unwrap().0;
    DocByte(i as u32)
}

/// A bookmark on the only "Same." paragraph; then the author inserts another
/// "Same." paragraph ABOVE it. The bookmark should stay on its paragraph
/// (the one followed by "Target body"), not jump to the new one.
#[test]
fn a_copy_inserted_above_does_not_take_a_kept_place_over() {
    let before = "# Doc\n\nIntro.\n\nSame.\n\nTarget body here.\n";
    let doc = Document::parse(before);
    let d = at(&doc, "Same.", 0);
    let p = Place::kept(&doc, d);
    let after = "# Doc\n\nSame.\n\nIntro.\n\nSame.\n\nTarget body here.\n";
    let doc2 = Document::parse(after);
    let got = p.resolve(&doc2);
    let want = at(&doc2, "Same.", 1);
    assert_eq!(got, want, "landed at {:?}", &doc2.text[got.0 as usize..]);
}

/// A place in the SECOND of two identical paragraphs; the second is deleted
/// (and one far away remains). The doc says an edited block falls back to the
/// source place; instead it jumps to the other copy far above.
#[test]
fn a_kept_place_whose_copy_is_deleted_stays_where_it_was() {
    let filler = "Para.\n\n".repeat(50);
    let before = format!("Same.\n\n{filler}Near.\n\nSame.\n\nEnd.\n");
    let doc = Document::parse(&before);
    let d = at(&doc, "Same.", 1);
    let p = Place::kept(&doc, d);
    let after = format!("Same.\n\n{filler}Near.\n\nEnd.\n");
    let doc2 = Document::parse(&after);
    let got = p.resolve(&doc2);
    assert!(
        got.0 as usize > doc2.text.find("Near.").unwrap(),
        "jumped to {:?}",
        &doc2.text[got.0 as usize..(got.0 as usize + 10).min(doc2.text.len())]
    );
}
