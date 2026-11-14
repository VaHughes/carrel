use carrel_core::{DocByte, Document};

/// Old Mac line endings: the parser reads each `\r` as a line break, so
/// line numbers must too, or every hit is "line 1" quoting the whole file.
#[test]
fn a_lone_cr_ends_a_line_for_line_numbers_too() {
    let src = "alpha\r\rbeta\r\rthe needle here\r";
    let doc = Document::parse(src);
    let at = DocByte(doc.text.find("needle").unwrap() as u32);
    assert_eq!(doc.line_of(at), 5);
    assert_eq!(
        doc.line_start(5),
        DocByte(doc.text.find("the needle").unwrap() as u32)
    );
    // CRLF counts once, LF once.
    let doc = Document::parse("a\r\n\r\nb\n\nc\n");
    assert_eq!(doc.line_of(DocByte(doc.text.find('c').unwrap() as u32)), 5);
    assert_eq!(
        doc.line_start(3),
        DocByte(doc.text.find('b').unwrap() as u32)
    );
}
