//! Where a word may be divided, and whether this document's words should be.
//!
//! A narrow column of unhyphenated English is ragged: every long word that
//! misses the end of a row leaves a hole the width of itself. Dividing the
//! word fills the hole. This module answers the two questions that takes —
//! *where* a word may be divided, and *whether* the words in front of it are
//! ones the answer applies to — and nothing else. Fitting is [`crate::layout`]'s.
//!
//! # The hyphen is never in the text
//!
//! [`Document::text`] is authoritative, so a division is a fact about a ROW —
//! [`RowKind::Text::hyphen`](crate::layout::RowKind) — and the glyph is the
//! frontend's decoration, exactly like a bullet or a quote bar. Search,
//! selection and copy run over bytes that contain no hyphen, which is why a
//! match spanning a division still matches and a copied paragraph pastes as
//! the words that were written.
//!
//! # Where: Liang's patterns
//!
//! The TeX algorithm, over the American English pattern set TeX ships
//! (`hyph-en-us.pat.txt`, embedded with its notice; 31 KiB). No dependency:
//! the algorithm is forty lines and the data is the whole cost.
//!
//! # Whether: the rules are narrow on purpose
//!
//! A wrong hyphen is worse than a ragged edge, so everything here fails
//! toward *not dividing*:
//!
//! - **English patterns divide English words only.** They are not "roughly
//!   right" for German; they are wrong. [`hyphenation_for`] honours a `lang:`
//!   in the frontmatter and otherwise asks whether the prose reads as English
//!   ([`reads_as_english`]); anything else is left alone.
//! - **Only a free-standing, all-lowercase ASCII word divides**
//!   ([`word_in`]). That one rule excludes identifiers, paths, URLs, numbers,
//!   acronyms, proper nouns and every word the patterns were not built for.
//! - **Never inside code, math or a script run** — a hyphen there reads as
//!   part of the name. The layout checks the style runs.

use std::ops::Range;
use std::sync::LazyLock;

use crate::document::{Document, NodeKind, Style};

/// Whether, and in which language, a word too long for the rest of its row
/// may be divided.
///
/// A language, not a boolean, because the patterns are per-language and the
/// honest answer for a language with no patterns here is [`Self::Off`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Hyphenation {
    #[default]
    Off,
    /// American English patterns.
    English,
}

/// Rows at least this wide are never hyphenated.
///
/// Raggedness is a property of the ratio between word length and row length:
/// at a full reading measure a missed word leaves a hole nobody notices, and
/// a hyphen there is noise. Tested against the space a row actually has, not
/// the terminal's width — a list item nested four deep in a wide window is a
/// narrow column too.
pub const NARROW_BELOW: u16 = 70;

/// Letters that must stay before a division. TeX's value for these patterns.
const LEFT_MIN: usize = 2;
/// Letters that must follow a division. TeX's value for these patterns.
const RIGHT_MIN: usize = 3;
/// Shortest word divided. `to-day` is a division the patterns permit and a
/// reader trips on: under six letters the halves are too small to read as
/// parts of a word, and the hole a five-letter word leaves is small anyway.
const MIN_WORD: usize = 6;
/// Longest word considered. The break set is a `u64` bitmask, and a longer
/// "word" is not prose.
const MAX_WORD: usize = 48;

const PATTERNS: &str = include_str!("hyph-en-us.pat.txt");

/// TeX's exception list for these patterns (`hyph-en-us.hyp.txt`), verbatim.
/// A word here divides only where marked, whatever the patterns say.
const EXCEPTIONS: [&str; 14] = [
    "as-so-ciate",
    "as-so-ciates",
    "dec-li-na-tion",
    "oblig-a-tory",
    "phil-an-thropic",
    "present",
    "presents",
    "project",
    "projects",
    "reci-procity",
    "re-cog-ni-zance",
    "ref-or-ma-tion",
    "ret-ri-bu-tion",
    "ta-ble",
];

/// `.` (a word edge) and the 26 letters.
const ALPHABET: usize = 27;

#[derive(Clone)]
struct TrieNode {
    /// Child per letter code; 0 is "none" (the root is never a child).
    next: [u16; ALPHABET],
    /// This node's pattern levels, as a slice of [`Trie::levels`]. Empty when
    /// no pattern ends here.
    levels: Range<u32>,
}

/// The patterns as a letter trie, so a word is scored by walking it once per
/// starting letter rather than hashing every substring.
struct Trie {
    nodes: Vec<TrieNode>,
    levels: Vec<u8>,
}

fn code(b: u8) -> usize {
    if b == b'.' {
        0
    } else {
        usize::from(b - b'a') + 1
    }
}

impl Trie {
    fn build(patterns: &str) -> Self {
        let blank = TrieNode {
            next: [0; ALPHABET],
            levels: 0..0,
        };
        let mut t = Self {
            nodes: vec![blank.clone()],
            levels: Vec::new(),
        };
        for line in patterns.lines() {
            if line.is_empty() || line.starts_with('%') {
                continue;
            }
            // `a1bc3d` is the letters `abcd` with level 1 before `b` and
            // level 3 before `d`: one level per gap, letters + 1 of them.
            let mut at = 0usize;
            let mut levels = vec![0u8];
            for b in line.bytes() {
                if b.is_ascii_digit() {
                    *levels.last_mut().expect("never empty") = b - b'0';
                    continue;
                }
                let c = code(b);
                if t.nodes[at].next[c] == 0 {
                    t.nodes[at].next[c] =
                        u16::try_from(t.nodes.len()).expect("the pattern trie fits in u16");
                    t.nodes.push(blank.clone());
                }
                at = usize::from(t.nodes[at].next[c]);
                levels.push(0);
            }
            let start = t.levels.len() as u32;
            t.levels.extend_from_slice(&levels);
            t.nodes[at].levels = start..t.levels.len() as u32;
        }
        t
    }
}

static TRIE: LazyLock<Trie> = LazyLock::new(|| Trie::build(PATTERNS));

/// Where `word` may be divided, as a bitmask: bit `p` set means a hyphen may
/// go before byte `p`.
///
/// `word` must be ASCII lowercase letters; anything else, and any word too
/// short or too long to divide, answers 0.
#[must_use]
fn points(word: &str) -> u64 {
    let n = word.len();
    if !(MIN_WORD..=MAX_WORD).contains(&n) || !word.bytes().all(|b| b.is_ascii_lowercase()) {
        return 0;
    }
    let allowed = |p: usize| (LEFT_MIN..=n - RIGHT_MIN).contains(&p);

    for ex in EXCEPTIONS {
        if ex.len() >= n && ex.bytes().filter(|&b| b != b'-').eq(word.bytes()) {
            let mut mask = 0u64;
            let mut p = 0usize;
            for b in ex.bytes() {
                if b == b'-' {
                    if allowed(p) {
                        mask |= 1 << p;
                    }
                } else {
                    p += 1;
                }
            }
            return mask;
        }
    }

    // `.word.` — the dots are what let a pattern anchor to a word edge.
    let mut buf = [0u8; MAX_WORD + 2];
    buf[0] = b'.';
    buf[1..=n].copy_from_slice(word.as_bytes());
    buf[n + 1] = b'.';
    let buf = &buf[..n + 2];
    // One level per gap, the gap before `buf[i]` at index `i`.
    let mut score = [0u8; MAX_WORD + 3];

    let trie = &*TRIE;
    for i in 0..buf.len() {
        let mut at = 0usize;
        for &b in &buf[i..] {
            at = usize::from(trie.nodes[at].next[code(b)]);
            if at == 0 {
                break;
            }
            let r = &trie.nodes[at].levels;
            for (k, &lv) in trie.levels[r.start as usize..r.end as usize]
                .iter()
                .enumerate()
            {
                score[i + k] = score[i + k].max(lv);
            }
        }
    }

    // An odd level permits a division. Word byte `p` is `buf[p + 1]`.
    let mut mask = 0u64;
    for p in 1..n {
        if allowed(p) && score[p + 1] % 2 == 1 {
            mask |= 1 << p;
        }
    }
    mask
}

/// Punctuation that may open a word without making it something else.
const OPENERS: &[char] = &['(', '[', '"', '\'', '\u{201C}', '\u{2018}'];
/// Punctuation that may close one.
const CLOSERS: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '"', '\'', '\u{201D}', '\u{2019}', '\u{2026}',
];

/// The divisible word inside one break unit's content, as a byte range of
/// `content` — or `None`, which is the answer for almost everything that is
/// not an ordinary English word.
///
/// The shape is: optional opening punctuation, a run of **ASCII lowercase**
/// letters, an optional possessive, optional closing punctuation, and nothing
/// else. So `(reflowing),` qualifies and none of these do:
///
/// - `snake_case`, `main.rs`, `https://…`, `v2`, `a/b` — not a letter run.
/// - `README`, `JavaScript`, `Carrel` — any capital. Acronyms and identifiers
///   must not divide, and a proper noun should not; the cost is that the
///   first word of a sentence does not either.
/// - `naïve`, `café` — the patterns are ASCII, so their answer for a word
///   they cannot spell is not an answer.
/// - `doesn’t` — a contraction is not a word followed by punctuation.
#[must_use]
pub(crate) fn word_in(content: &str) -> Option<Range<usize>> {
    let start = content.len() - content.trim_start_matches(OPENERS).len();
    let letters = content[start..]
        .bytes()
        .take_while(u8::is_ascii_lowercase)
        .count();
    let end = start + letters;
    let mut rest = &content[end..];
    for possessive in ["'s", "\u{2019}s"] {
        if let Some(r) = rest.strip_prefix(possessive) {
            rest = r;
            break;
        }
    }
    if letters == 0 || !rest.trim_start_matches(CLOSERS).is_empty() {
        return None;
    }
    Some(start..end)
}

/// The latest division of `word` that leaves at most `max_head` letters
/// before the hyphen, as a byte offset into `word`.
#[must_use]
pub(crate) fn division(word: &str, max_head: usize) -> Option<usize> {
    let mask = points(word);
    (1..=max_head.min(MAX_WORD))
        .rev()
        .find(|&p| mask & (1 << p) != 0)
}

/// How many prose words [`reads_as_english`] looks at. The answer is settled
/// long before this, and a bound keeps the open path flat on a large document.
const SAMPLE_WORDS: u32 = 2000;

/// Words common in English and rare, as whole words, in the languages it is
/// most often mistaken for. `in`, `is`, `was`, `for`, `to`, `of` and `a` are
/// deliberately absent: each is a common word of some other language (German
/// and Dutch `in`, Dutch `is`/`of`, German `was`, Danish `for`, Polish `to`,
/// Spanish `a`).
const ENGLISH: [&str; 14] = [
    "the", "and", "that", "with", "this", "from", "are", "not", "you", "which", "have", "when",
    "will", "can",
];

/// Does this document's prose read as English?
///
/// Counts function words over the first [`SAMPLE_WORDS`] words of running
/// prose (paragraphs, list items, definitions — never code or tables).
/// Measured 2026-09-30: nine English documents, from terse status notes to a
/// tutorial, spend between 10% and 18% of their words on the list above;
/// Wikipedia articles in thirteen other languages — German, Dutch, French,
/// Spanish, Italian, Portuguese, Swedish, Danish, Norwegian, Polish, Czech,
/// Finnish, Afrikaans — spend at most 0.2%. The threshold is one word in
/// sixteen and at least five of them, with barely a word in fifty holding a
/// letter English lacks; a text too short to say either way
/// answers **no**: the failure mode is an
/// unhyphenated paragraph, which is what every reader had before.
///
/// A document that mixes languages is judged as a whole. The non-ASCII test
/// keeps most mixtures out, but a language written almost wholly in ASCII
/// (Dutch, Indonesian) mixed with enough English can still be judged English,
/// and English patterns then reach its words. `lang:` in the frontmatter is
/// the answer to that, and it wins — see [`hyphenation_for`].
#[must_use]
pub fn reads_as_english(doc: &Document) -> bool {
    let (hits, words, foreign) = english_share(doc);
    // Enough function words, and enough of them to mean it — and almost no
    // word with a letter English does not have. The second test is for the
    // document the first cannot see: mostly German with one English
    // sentence quoted in it is dense in `the` and `that`, and also full of
    // `ü` and `ß`. One word, and one more per fifty, leaves room for a café.
    hits >= 5 && hits * 16 >= words && foreign <= 1 + words / 50
}

/// `(function words, words, words with a non-ASCII letter)` over the
/// sampled prose.
fn english_share(doc: &Document) -> (u32, u32, u32) {
    let (mut words, mut hits, mut foreign) = (0u32, 0u32, 0u32);
    'blocks: for id in &*doc.layout_order {
        let node = &doc.nodes[id.get()];
        if !divisible_kind(&node.kind) {
            continue;
        }
        // Prose only, which excludes the code INSIDE prose: SQL and Python
        // are written in English words, and a German page that names `FROM`
        // and `AND` in backticks is still German.
        let mut at = node.doc.start;
        let code = node
            .inlines
            .iter()
            .filter(|i| i.style.contains(Style::CODE) || i.style.contains(Style::MATH))
            .map(|i| i.doc.clone())
            .chain(std::iter::once(node.doc.end..node.doc.end));
        for skip in code {
            let text = &doc.text[at as usize..skip.start.max(at) as usize];
            at = skip.end.max(at);
            for word in text.split(|c: char| !c.is_alphabetic()) {
                if word.is_empty() {
                    continue;
                }
                words += 1;
                if ENGLISH.iter().any(|e| e.eq_ignore_ascii_case(word)) {
                    hits += 1;
                } else if !word.is_ascii() {
                    foreign += 1;
                }
                if words >= SAMPLE_WORDS {
                    break 'blocks;
                }
            }
        }
    }
    (hits, words, foreign)
}

/// The language a document's frontmatter declares, as "is it English?".
///
/// `lang:` or `language:` in a YAML block, `lang = "…"` in a TOML one. Not a
/// YAML parse — a reader must never fail on exotic YAML — so this reads the
/// one line it is looking for and nothing else.
fn declared_english(doc: &Document) -> Option<bool> {
    let node = doc
        .layout_order
        .first()
        .map(|id| &doc.nodes[id.get()])
        .filter(|n| matches!(n.kind, NodeKind::Metadata { .. }))?;
    let body = &doc.text[node.doc.start as usize..node.doc.end as usize];
    body.lines().find_map(|line| {
        let (key, value) = line.split_once([':', '='])?;
        if !matches!(key.trim(), "lang" | "language") {
            return None;
        }
        let value = value.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        let tag = value.split(['-', '_']).next().unwrap_or_default();
        Some(matches!(tag, "en" | "eng" | "english"))
    })
}

/// Block kinds whose text is running prose: the ones that may be hyphenated,
/// and the ones the language is judged from.
///
/// **Exhaustive on purpose**, like the budget match in the frontend's layout:
/// a new kind must be classified deliberately. A heading is prose but is not
/// here — a divided heading is a typographic fault in every style guide, and
/// headings are short enough to take the ragged edge.
pub(crate) fn divisible_kind(kind: &NodeKind) -> bool {
    match kind {
        NodeKind::Paragraph | NodeKind::Item | NodeKind::DefDetails => true,
        NodeKind::Root
        | NodeKind::List { .. }
        | NodeKind::BlockQuote
        | NodeKind::Heading { .. }
        | NodeKind::CodeBlock { .. }
        | NodeKind::AlertLabel { .. }
        | NodeKind::Table { .. }
        | NodeKind::Math
        | NodeKind::DefTerm
        | NodeKind::Metadata { .. }
        | NodeKind::Image { .. }
        | NodeKind::Rule => false,
    }
}

/// How this document's prose should be hyphenated: by what its frontmatter
/// declares, else by what its prose reads as.
///
/// A declared language always wins, in both directions — `lang: de` turns
/// hyphenation off for a document that quotes a great deal of English, and
/// `lang: en` turns it on for one too short to judge.
#[must_use]
pub fn hyphenation_for(doc: &Document) -> Hyphenation {
    let english = declared_english(doc).unwrap_or_else(|| reads_as_english(doc));
    if english {
        Hyphenation::English
    } else {
        Hyphenation::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `word` with a `-` at every permitted division.
    fn divided(word: &str) -> String {
        let mask = points(word);
        let mut out = String::new();
        for (p, ch) in word.char_indices() {
            if mask & (1 << p) != 0 {
                out.push('-');
            }
            out.push(ch);
        }
        out
    }

    #[test]
    fn the_patterns_divide_words_where_tex_does() {
        // Liang's own worked example, then TeX's published divisions.
        assert_eq!(divided("hyphenation"), "hy-phen-ation");
        assert_eq!(divided("computer"), "com-puter");
        assert_eq!(divided("algorithm"), "al-go-rithm");
        assert_eq!(
            divided("supercalifragilisticexpialidocious"),
            "su-per-cal-ifrag-ilis-tic-ex-pi-ali-do-cious"
        );
    }

    #[test]
    fn an_exception_overrides_the_patterns() {
        assert_eq!(divided("associate"), "as-so-ciate");
        assert_eq!(divided("reformation"), "ref-or-ma-tion");
        assert_eq!(divided("project"), "project", "listed with no division");
        assert_eq!(divided("present"), "present");
        assert_eq!(divided("tables"), "ta-bles", "an inflection is not listed");
    }

    #[test]
    fn a_division_always_leaves_two_letters_before_and_three_after() {
        for word in ["hyphenation", "reader", "elephant", "associate", "ideals"] {
            let mask = points(word);
            for p in 0..word.len() {
                if mask & (1 << p) != 0 {
                    assert!(p >= LEFT_MIN, "{word}: {p} letters before");
                    assert!(word.len() - p >= RIGHT_MIN, "{word}: after {p}");
                }
            }
        }
    }

    #[test]
    fn words_the_patterns_were_not_built_for_never_divide() {
        // `today` and `table` are divisions TeX makes (`to-day`, `ta-ble`)
        // and this reader declines: too short to be worth a hyphen.
        for word in [
            "",
            "the",
            "into",
            "today",
            "table",
            "README",
            "Carrel",
            "naïve",
            "snake_case",
        ] {
            assert_eq!(points(word), 0, "{word:?}");
        }
        let long = "a".repeat(MAX_WORD + 1);
        assert_eq!(points(&long), 0, "past the bitmask");
        // At the limit the scan must stay inside its buffers.
        let _ = points(&"ab".repeat(MAX_WORD / 2));
    }

    #[test]
    fn a_word_is_lowercase_letters_with_punctuation_only_at_its_edges() {
        let word = |s: &str| word_in(s).map(|r| s[r].to_string());
        assert_eq!(word("reflowing").as_deref(), Some("reflowing"));
        assert_eq!(word("(reflowing),").as_deref(), Some("reflowing"));
        assert_eq!(
            word("\u{201C}reflowing.\u{201D}").as_deref(),
            Some("reflowing")
        );
        assert_eq!(word("maintainer\u{2019}s").as_deref(), Some("maintainer"));
        for not in [
            "snake_case",
            "main.rs",
            "https://example.com/path",
            "camelCase",
            "README",
            "Carrel",
            "naïve",
            "doesn\u{2019}t",
            "well-known",
            "v2",
            "a/b",
            "...",
            "",
        ] {
            assert_eq!(word(not), None, "{not:?} must not be divisible");
        }
    }

    #[test]
    fn the_latest_division_that_fits_is_chosen() {
        // hy-phen-ation
        assert_eq!(division("hyphenation", 10), Some(6));
        assert_eq!(division("hyphenation", 6), Some(6));
        assert_eq!(division("hyphenation", 5), Some(2));
        assert_eq!(division("hyphenation", 1), None);
        assert_eq!(division("project", 6), None);
    }

    const ENGLISH_PROSE: &str = "Carrel reads only the folder you point it at. \
        Anything wider is a root that you choose yourself, and the index it caches \
        holds file paths and modification times. Links resolving inside your folder \
        follow as they always have, and one resolving outside names the path.";

    #[test]
    fn english_prose_reads_as_english() {
        assert!(reads_as_english(&Document::parse(ENGLISH_PROSE)));
    }

    #[test]
    fn other_languages_do_not() {
        for (lang, text) in [
            (
                "German",
                "Der schnelle braune Fuchs springt über den faulen Hund, und die \
                 Geschwindigkeit der Verarbeitung ist in diesem Dokument nicht \
                 wichtig. Was wir hier sehen, ist ein Text mit vielen Wörtern, der \
                 von einem Programm gelesen wird, das keine Ahnung von Deutsch hat.",
            ),
            (
                "Dutch",
                "De snelle bruine vos springt over de luie hond, en de snelheid van \
                 de verwerking is in dit document niet belangrijk. Wat we hier zien \
                 is een tekst met veel woorden, of een tekst die door een programma \
                 wordt gelezen dat geen verstand heeft van het Nederlands.",
            ),
            (
                "French",
                "Le renard brun rapide saute par-dessus le chien paresseux, et la \
                 vitesse du traitement est sans importance dans ce document. Ce que \
                 nous voyons ici est un texte avec beaucoup de mots, lu par un \
                 programme qui ne comprend pas le français.",
            ),
            (
                "Spanish",
                "El rápido zorro marrón salta sobre el perro perezoso, y la velocidad \
                 del procesamiento no es importante en este documento. Lo que vemos \
                 aquí es un texto con muchas palabras, leído por un programa que no \
                 entiende el español.",
            ),
            (
                "Danish",
                "Den hurtige brune ræv springer over den dovne hund, og hastigheden \
                 for behandlingen er ikke vigtig i dette dokument. Det vi ser her er \
                 en tekst med mange ord, som vi have læst for at forstå, og som et \
                 program ikke kan læse.",
            ),
        ] {
            assert!(
                !reads_as_english(&Document::parse(text)),
                "{lang} prose must not be hyphenated with English patterns"
            );
        }
    }

    #[test]
    fn keywords_in_inline_code_are_not_english_prose() {
        // SQL and Python are written in English words. A German page about
        // them is still German, and must not get English hyphenation.
        let src = "Die Abfrage beginnt mit `SELECT` und `FROM`, danach folgen \
            `WHERE`, `AND`, `NOT` und `WITH`. Das Schlüsselwort `WHEN` steht in \
            einer Fallunterscheidung, und `this` verweist auf das aktuelle \
            Objekt. Verschiedene Datenbanken funktionieren hier unterschiedlich, \
            und die benannten Parameter sind nicht überall verfügbar. Mit `from` \
            und `with` werden in Python Module und Kontexte geöffnet, `not` und \
            `and` verknüpfen Bedingungen, und `that` ist kein Schlüsselwort.";
        let doc = Document::parse(src);
        assert!(!reads_as_english(&doc));
        // The control: the same keywords as prose words do count.
        let (hits, ..) = english_share(&Document::parse(&src.replace('`', "")));
        assert!(hits >= 10, "the fixture must contain the words: {hits}");
    }

    #[test]
    fn a_document_that_quotes_some_english_is_not_english() {
        // One English sentence is dense enough in function words to carry a
        // short German note past the first test on its own.
        let src = "Wir haben die Anleitung gelesen und verschiedene Möglichkeiten \
            geprüft. Der wichtigste Satz lautet: \"This is the part that you \
            will have read when the release notes are not with you.\" Danach \
            wird erklärt, wie die Einstellungen geändert werden können und \
            welche Folgen das für die übrigen Geräte hat.";
        let doc = Document::parse(src);
        let (hits, words, _) = english_share(&doc);
        assert!(
            hits >= 5 && hits * 16 >= words,
            "the fixture must pass the first test"
        );
        assert!(!reads_as_english(&doc));
    }

    #[test]
    fn a_borrowed_word_does_not_make_english_foreign() {
        let src = format!("{ENGLISH_PROSE} The café is closed and the résumé is late.");
        assert!(reads_as_english(&Document::parse(&src)));
    }

    #[test]
    fn a_text_too_short_to_judge_is_not_english() {
        assert!(!reads_as_english(&Document::parse("")));
        assert!(!reads_as_english(&Document::parse(
            "Release notes and the plan"
        )));
    }

    #[test]
    fn code_does_not_count_toward_the_language() {
        // English keywords inside a fence are not English prose.
        let src = "```\nthe and that with this from are not you which have\n```\n";
        assert!(!reads_as_english(&Document::parse(src)));
    }

    #[test]
    fn a_declared_language_wins_in_both_directions() {
        let de = format!("---\nlang: de\n---\n\n{ENGLISH_PROSE}");
        assert_eq!(hyphenation_for(&Document::parse(&de)), Hyphenation::Off);
        let en = "---\ntitle: Notes\nlang: en-GB\n---\n\nShort.";
        assert_eq!(hyphenation_for(&Document::parse(en)), Hyphenation::English);
        let toml = "+++\nlanguage = \"English\"\n+++\n\nShort.";
        assert_eq!(
            hyphenation_for(&Document::parse(toml)),
            Hyphenation::English
        );
        let undeclared = format!("---\ntitle: Notes\n---\n\n{ENGLISH_PROSE}");
        assert_eq!(
            hyphenation_for(&Document::parse(&undeclared)),
            Hyphenation::English,
            "frontmatter without a language falls back to the prose"
        );
    }
}
