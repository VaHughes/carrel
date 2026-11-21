//! A bare URL beside prose that smart punctuation rewrites: the address a
//! link copies, and the dashes it shows, are the ones typed in THAT URL.
//!
//! Smart punctuation turns `--` into a dash, `...` into an ellipsis and
//! curls quotes, in a URL as anywhere. Where the reader cannot pair the
//! shown URL with one in the source — an entity in it — it used to decide
//! what had been typed by looking around the paragraph, and a `1–2` typed
//! in the prose vouched for the `1--2` in the address.

use carrel_core::Document;
use proptest::prelude::*;

/// What a URL's path may be made of: each kind of smart punctuation, a
/// dash typed as a dash, and the entity that keeps a URL from pairing.
const SEGMENTS: &[&str] = &[
    "x",
    "1--2",
    "1\u{2013}2",
    "a---b",
    "a\u{2014}b",
    "10--20",
    "p...q",
    "it's",
    "?k&amp;v",
    "&amp;",
    "-",
    ".",
    "/",
];

/// Prose around the URLs: the same characters, where they mean nothing to
/// any address.
const PROSE: &[&str] = &[
    "word",
    "1\u{2013}2",
    "1--2",
    "10\u{2014}20",
    "a---b",
    "and...",
    "it's",
    "\"quoted\"",
    "\u{2014}",
    "--",
    "Q&amp;A",
    "(see",
    "here)",
];

fn url() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec!["https://a.b/", "http://c.d/", "www.e.f/"]),
        prop::collection::vec(prop::sample::select(SEGMENTS), 1..5),
    )
        .prop_map(|(scheme, segments)| format!("{scheme}{}z", segments.concat()))
}

/// A URL, or a word of prose.
fn piece() -> impl Strategy<Value = (bool, String)> {
    prop_oneof![
        url().prop_map(|u| (true, u)),
        prop::sample::select(PROSE).prop_map(|p| (false, p.to_string())),
    ]
}

/// The address typed: the entity decoded, `www.` behind its scheme.
fn address(typed: &str) -> String {
    let decoded = typed.replace("&amp;", "&");
    if decoded.starts_with("www.") {
        format!("http://{decoded}")
    } else {
        decoded
    }
}

/// What is shown for it: its dashes as typed. An ellipsis and a curled
/// quote are shown as in any prose, so both are read as their ASCII here.
fn shown(typed: &str) -> String {
    typed.replace("&amp;", "&")
}

fn plain_quotes(text: &str) -> String {
    text.replace('\u{2026}', "...")
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]
    #[test]
    fn every_bare_url_is_copied_and_shown_as_typed(
        pieces in prop::collection::vec(piece(), 1..7)
    ) {
        let src = pieces
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let doc = Document::parse(&format!("{src}\n"));
        let urls: Vec<&String> = pieces.iter().filter(|(is, _)| *is).map(|(_, u)| u).collect();
        let want: Vec<String> = urls.iter().map(|u| address(u)).collect();
        let links: Vec<&str> = doc.links.iter().map(|l| &**l).collect();
        prop_assert_eq!(&links, &want, "{:?} shows {:?}", src, doc.text);
        // Shown in order, each with its own dashes.
        let text = plain_quotes(&doc.text);
        let mut from = 0;
        for url in urls {
            let shown = shown(url);
            let at = text[from..].find(&shown);
            prop_assert!(at.is_some(), "{:?} not shown in {:?} (from {:?})", shown, text, src);
            from += at.unwrap() + shown.len();
        }
    }
}
