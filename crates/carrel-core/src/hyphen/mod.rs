//! Where a word may be divided, and whether this document's words should be.
//!
//! A narrow column of unhyphenated prose is ragged: every long word that
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
//! The TeX algorithm, over the pattern sets TeX ships in hyph-utf8, embedded
//! with their notices: American English, and nine more a document can ask
//! for by name ([`Hyphenation::from_tag`]). Each was chosen because its
//! licence lets it be embedded in an `MIT OR Apache-2.0` binary as it stands
//! — MIT, BSD, or the all-permissive notice. Each file is hyph-utf8's own,
//! unmodified, under a `%` header carrying the upstream commit and the .tex
//! source's copyright and licence verbatim; THIRD-PARTY-NOTICES at the
//! repository root collects them, ships beside every binary, and is why the
//! crates' licence expression is more than `MIT OR Apache-2.0`. Swedish and
//! Russian are not here because their
//! patterns are LPPL-only. No dependency: the algorithm is forty lines and
//! the data is the whole cost.
//!
//! # Whether: the rules are narrow on purpose
//!
//! A wrong hyphen is worse than a ragged edge, so everything here fails
//! toward *not dividing*:
//!
//! - **A language's patterns divide that language's words only.** English
//!   patterns are not "roughly right" for German; they are wrong. A language
//!   is never guessed from the words, with one exception kept from before
//!   there were others: a document that declares nothing is hyphenated as
//!   English when its prose reads as English ([`reads_as_english`]). A
//!   declared language the patterns here do not cover turns hyphenation
//!   OFF — it never falls back to English. See [`hyphenation_for`].
//! - **Only a free-standing lowercase word divides** ([`word_in`]), spelled
//!   wholly in its language's alphabet. That one rule excludes identifiers,
//!   paths, URLs, numbers, acronyms, proper nouns and every word the patterns
//!   were not built for. German, where every noun is capitalized, is the one
//!   language whose words may open with a capital.
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
    /// American English patterns (`en`, and every `en-*`).
    English,
    /// German, reformed orthography (`de`; not `de-1901`).
    German,
    /// French (`fr`).
    French,
    /// Spanish (`es`).
    Spanish,
    /// Italian (`it`).
    Italian,
    /// Portuguese (`pt`, `pt-BR`).
    Portuguese,
    /// Dutch (`nl`).
    Dutch,
    /// Danish (`da`).
    Danish,
    /// Norwegian, Bokmål and Nynorsk alike (`no`, `nb`, `nn`).
    Norwegian,
    /// Polish (`pl`).
    Polish,
}

impl Hyphenation {
    /// Every language with patterns, in no particular order.
    pub const LANGUAGES: [Self; 10] = [
        Self::English,
        Self::German,
        Self::French,
        Self::Spanish,
        Self::Italian,
        Self::Portuguese,
        Self::Dutch,
        Self::Danish,
        Self::Norwegian,
        Self::Polish,
    ];

    /// The hyphenation a language tag asks for.
    ///
    /// A BCP 47 tag (`de`, `fr-CA`, `pt-BR`, `nb-NO`), an ISO 639-2 or -3
    /// code (`deu`, `fre`), or the language's name in English or in itself
    /// (`German`, `Deutsch`), in any case. Only the primary subtag decides,
    /// with one exception: German's `1901` variant is the traditional
    /// orthography, which the 1996 patterns divide wrongly (`ck`, `st`), so
    /// it is [`Self::Off`].
    ///
    /// `None` when the value names nothing at all — an empty `lang:` is not a
    /// declaration. Anything else this does not recognize is [`Self::Off`]:
    /// a document that says what it is written in is never hyphenated as
    /// something else.
    #[must_use]
    pub fn from_tag(value: &str) -> Option<Self> {
        let tag = value.trim().trim_matches(['"', '\'']).trim().to_lowercase();
        if tag.is_empty() {
            return None;
        }
        let mut subtags = tag.split(['-', '_']);
        let primary = subtags.next().unwrap_or_default();
        let traditional = subtags.any(|s| s == "1901");
        Some(match primary {
            "en" | "eng" | "english" => Self::English,
            "de" | "deu" | "ger" | "german" | "deutsch" if !traditional => Self::German,
            "fr" | "fra" | "fre" | "french" | "français" | "francais" => Self::French,
            "es" | "spa" | "spanish" | "español" | "espanol" | "castellano" => Self::Spanish,
            "it" | "ita" | "italian" | "italiano" => Self::Italian,
            "pt" | "por" | "portuguese" | "português" | "portugues" => Self::Portuguese,
            "nl" | "nld" | "dut" | "dutch" | "nederlands" => Self::Dutch,
            "da" | "dan" | "danish" | "dansk" => Self::Danish,
            "no" | "nor" | "nb" | "nob" | "nn" | "nno" | "norwegian" | "norsk" | "bokmål"
            | "bokmal" | "nynorsk" => Self::Norwegian,
            "pl" | "pol" | "polish" | "polski" => Self::Polish,
            _ => Self::Off,
        })
    }

    fn patterns(self) -> Option<&'static Patterns> {
        Some(match self {
            Self::Off => return None,
            Self::English => &EN,
            Self::German => &DE,
            Self::French => &FR,
            Self::Spanish => &ES,
            Self::Italian => &IT,
            Self::Portuguese => &PT,
            Self::Dutch => &NL,
            Self::Danish => &DA,
            Self::Norwegian => &NO,
            Self::Polish => &PL,
        })
    }
}

/// Rows at least this wide are never hyphenated.
///
/// Raggedness is a property of the ratio between word length and row length:
/// at a full reading measure a missed word leaves a hole nobody notices, and
/// a hyphen there is noise. Tested against the space a row actually has, not
/// the terminal's width — a list item nested four deep in a wide window is a
/// narrow column too.
pub const NARROW_BELOW: u16 = 70;

/// Shortest word divided, in letters. `to-day` is a division the English
/// patterns permit and a reader trips on: under six letters the halves are
/// too small to read as parts of a word, and the hole a five-letter word
/// leaves is small anyway.
const MIN_WORD: usize = 6;
/// Longest word considered, in letters. The break set is a `u64` bitmask.
/// German compounds run long; anything longer than this is not prose.
pub(crate) const MAX_WORD: usize = 63;

/// One language's patterns, exceptions and conventions.
struct Patterns {
    /// Letters that must stay before a division: the language's own
    /// `\lefthyphenmin`, from the pattern file's header.
    left: usize,
    /// Letters that must follow one: its `\righthyphenmin`.
    right: usize,
    /// Whether a word may open with a capital. Only German's: every German
    /// noun is capitalized, so the lowercase rule would exclude exactly the
    /// long compounds hyphenation exists for. Everywhere else a capital
    /// marks a proper noun or a sentence's first word, and neither divides.
    capitalized: bool,
    /// Whether `'s` may close a word (`maintainer's`). English only.
    possessive: bool,
    /// `hyph-*.pat.txt`, verbatim: its notice in `%` lines at the top, one
    /// pattern per line below them.
    pat: &'static str,
    /// `hyph-*.hyp.txt` (words that divide only where marked), its notice in
    /// `%` lines too; empty for a language with no exceptions.
    hyp: &'static str,
    /// Both, built on first use, and only for the language a document asks for.
    trie: LazyLock<Trie>,
}

macro_rules! patterns {
    ($name:ident, $file:literal, $hyp:expr, $left:literal, $right:literal, $cap:literal, $pos:literal) => {
        static $name: Patterns = Patterns {
            left: $left,
            right: $right,
            capitalized: $cap,
            possessive: $pos,
            pat: include_str!(concat!("hyph-", $file, ".pat.txt")),
            hyp: $hyp,
            trie: LazyLock::new(|| Trie::build($name.pat, $name.hyp)),
        };
    };
}

// Licences, from each file's header (the files carry them in full, and
// THIRD-PARTY-NOTICES at the repository root collects them; the test
// `every_embedded_file_carries_its_licence` holds each file to it):
//   en-us — all-permissive notice (Kuiken) and Knuth's hyphen.tex notice
//   de-1996 — MIT · fr — MIT · es — MIT · nl — MIT
//   it, da — LPPL or MIT, taken under MIT · pl — MIT or Knuth's, taken under MIT
//   pt — BSD 3-clause · nb (no, nn) — all-permissive notice
patterns!(
    EN,
    "en-us",
    include_str!("hyph-en-us.hyp.txt"),
    2,
    3,
    false,
    true
);
patterns!(DE, "de-1996", "", 2, 2, true, false);
patterns!(FR, "fr", "", 2, 2, false, false);
patterns!(ES, "es", "", 2, 2, false, false);
patterns!(IT, "it", "", 2, 2, false, false);
patterns!(
    PT,
    "pt",
    include_str!("hyph-pt.hyp.txt"),
    2,
    3,
    false,
    false
);
patterns!(
    NL,
    "nl",
    include_str!("hyph-nl.hyp.txt"),
    2,
    2,
    false,
    false
);
patterns!(DA, "da", "", 2, 2, false, false);
patterns!(
    NO,
    "nb",
    include_str!("hyph-nb.hyp.txt"),
    2,
    2,
    false,
    false
);
patterns!(
    PL,
    "pl",
    include_str!("hyph-pl.hyp.txt"),
    2,
    2,
    false,
    false
);

/// The data lines of an embedded hyph-utf8 file, trimmed: everything but its
/// notice (`%` lines) and blank lines. One pattern per line in a `.pat.txt`;
/// whitespace-separated words in a `.hyp.txt`.
fn data_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('%'))
}

/// A pattern-trie node: its children, as a slice of [`Trie::edges`], and its
/// levels, as a slice of [`Trie::levels`] — empty when no pattern ends here.
struct Node {
    edges: Range<u32>,
    levels: Range<u32>,
}

/// The patterns as a letter trie, so a word is scored by walking it once per
/// starting letter rather than hashing every substring.
///
/// Children are a sorted slice per node rather than an array indexed by
/// letter: the alphabets differ per language and run past 26, and German's
/// 37,000 patterns as 40-wide arrays would be megabytes for a structure
/// asked a question once per row.
struct Trie {
    nodes: Vec<Node>,
    edges: Vec<(char, u32)>,
    levels: Vec<u8>,
    /// Every letter a pattern uses, sorted: the language's alphabet.
    alphabet: Vec<char>,
    /// The ASCII part of it as a bitmap, since every word asks per letter.
    ascii: u128,
    /// The root's children for ASCII letters; 0 is "none", as the root is
    /// never a child.
    root_ascii: [u32; 128],
    /// Exception words, as letters and the division points marked in them.
    exceptions: Vec<(Vec<char>, u64)>,
}

impl Trie {
    fn build(source: &str, exceptions: &str) -> Self {
        // Built with a child list per node, then flattened.
        let mut kids: Vec<Vec<(char, u32)>> = vec![Vec::new()];
        let mut ends: Vec<Range<u32>> = std::iter::once(0..0).collect();
        let mut levels = Vec::new();
        for line in data_lines(source) {
            // `a1bc3d` is the letters `abcd` with level 1 before `b` and
            // level 3 before `d`: one level per gap, letters + 1 of them.
            let mut at = 0usize;
            let mut gaps = vec![0u8];
            for c in line.chars() {
                if let Some(d) = c.to_digit(10) {
                    *gaps.last_mut().expect("never empty") = d as u8;
                    continue;
                }
                at = if let Some(&(_, n)) = kids[at].iter().find(|(k, _)| *k == c) {
                    n as usize
                } else {
                    let n = kids.len();
                    kids.push(Vec::new());
                    ends.push(0..0);
                    kids[at].push((c, n as u32));
                    n
                };
                gaps.push(0);
            }
            let start = levels.len() as u32;
            levels.extend_from_slice(&gaps);
            ends[at] = start..levels.len() as u32;
        }
        let mut alphabet = Vec::new();
        let mut edges = Vec::new();
        let mut nodes = Vec::with_capacity(kids.len());
        for (mut k, levels) in kids.into_iter().zip(ends) {
            k.sort_unstable_by_key(|&(c, _)| c);
            alphabet.extend(k.iter().map(|&(c, _)| c).filter(|&c| c != '.'));
            let from = edges.len() as u32;
            edges.extend(k);
            nodes.push(Node {
                edges: from..edges.len() as u32,
                levels,
            });
        }
        alphabet.sort_unstable();
        alphabet.dedup();
        let ascii = alphabet
            .iter()
            .filter(|c| c.is_ascii())
            .fold(0u128, |m, &c| m | 1 << u32::from(c));
        let mut root_ascii = [0u32; 128];
        let root = &nodes[0].edges;
        for &(c, n) in &edges[root.start as usize..root.end as usize] {
            if let Some(slot) = root_ascii.get_mut(u32::from(c) as usize) {
                *slot = n;
            }
        }
        let exceptions = data_lines(exceptions)
            .flat_map(str::split_whitespace)
            .map(|ex| {
                let (mut letters, mut mask) = (Vec::new(), 0u64);
                for c in ex.chars() {
                    if c == '-' {
                        if letters.len() < 64 {
                            mask |= 1 << letters.len();
                        }
                    } else {
                        letters.push(c);
                    }
                }
                (letters, mask)
            })
            .collect();
        Self {
            nodes,
            edges,
            levels,
            alphabet,
            ascii,
            root_ascii,
            exceptions,
        }
    }

    fn child(&self, at: usize, c: char) -> Option<usize> {
        // The root is asked once per letter of every word, so its ASCII
        // children are a table; below it, most nodes have a handful.
        if at == 0
            && let Some(&n) = self.root_ascii.get(u32::from(c) as usize)
        {
            return (n != 0).then_some(n as usize);
        }
        let r = &self.nodes[at].edges;
        let kids = &self.edges[r.start as usize..r.end as usize];
        if kids.len() <= 8 {
            return kids
                .iter()
                .find(|&&(k, _)| k == c)
                .map(|&(_, n)| n as usize);
        }
        kids.binary_search_by_key(&c, |&(k, _)| k)
            .ok()
            .map(|i| kids[i].1 as usize)
    }

    fn has(&self, c: char) -> bool {
        match u32::from(c) {
            a @ 0..128 => self.ascii & (1 << a) != 0,
            _ => self.alphabet.binary_search(&c).is_ok(),
        }
    }
}

impl Patterns {
    /// Where `word` may be divided, as a bitmask: bit `p` set means a hyphen
    /// may go before letter `p`.
    ///
    /// `word` must be lowercase letters of this language; a word too short
    /// or too long to divide answers 0.
    fn points(&self, word: &[char]) -> u64 {
        let n = word.len();
        if !(MIN_WORD..=MAX_WORD).contains(&n) {
            return 0;
        }
        let allowed = |p: usize| (self.left..=n - self.right).contains(&p);
        let trie = &*self.trie;

        if let Some((_, mask)) = trie.exceptions.iter().find(|(ex, _)| ex == word) {
            return (1..n)
                .filter(|&p| allowed(p) && mask & (1 << p) != 0)
                .fold(0, |m, p| m | 1 << p);
        }

        // `.word.` — the dots are what let a pattern anchor to a word edge.
        let mut buf = ['.'; MAX_WORD + 2];
        buf[1..=n].copy_from_slice(word);
        let buf = &buf[..n + 2];
        // One level per gap, the gap before `buf[i]` at index `i`.
        let mut score = [0u8; MAX_WORD + 3];
        for i in 0..buf.len() {
            let mut at = 0usize;
            for &c in &buf[i..] {
                let Some(next) = trie.child(at, c) else {
                    break;
                };
                at = next;
                let r = &trie.nodes[at].levels;
                for (k, &lv) in trie.levels[r.start as usize..r.end as usize]
                    .iter()
                    .enumerate()
                {
                    score[i + k] = score[i + k].max(lv);
                }
            }
        }

        // An odd level permits a division. Word letter `p` is `buf[p + 1]`.
        let mut mask = 0u64;
        for p in 1..n {
            if allowed(p) && score[p + 1] % 2 == 1 {
                mask |= 1 << p;
            }
        }
        mask
    }

    /// Whether `c` may stand at letter `i` of a word in this language.
    fn letter(&self, i: usize, c: char) -> bool {
        let trie = &*self.trie;
        if is_apostrophe(c) {
            // French and Italian patterns know elision (`l'organisation`);
            // the others have no apostrophe, so it ends their words.
            return i > 0 && trie.has(c);
        }
        if c.is_lowercase() {
            return trie.has(c);
        }
        i == 0 && self.capitalized && c.is_uppercase() && lowercase(c).is_some_and(|l| trie.has(l))
    }
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

/// The one lowercase letter `c` folds to, or `None` where it folds to more.
fn lowercase(c: char) -> Option<char> {
    if c.is_ascii() {
        return Some(c.to_ascii_lowercase());
    }
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// Punctuation that may open a word without making it something else.
/// The no-break spaces are there because French sets its guillemets off
/// with one, and the break unit then begins `«\u{A0}`.
const OPENERS: &[char] = &[
    '(', '[', '"', '\'', '\u{201C}', '\u{2018}', '\u{201E}', '\u{AB}', '\u{BB}', '\u{BF}',
    '\u{A1}', '\u{A0}', '\u{202F}',
];
/// Punctuation that may close one.
const CLOSERS: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '"', '\'', '\u{201D}', '\u{2019}', '\u{2026}',
    '\u{201C}', '\u{BB}', '\u{AB}', '\u{A0}', '\u{202F}',
];

/// The divisible word inside one break unit's content, as a byte range of
/// `content` — or `None`, which is the answer for almost everything that is
/// not an ordinary word of `language`.
///
/// The shape is: optional opening punctuation, a run of the language's
/// **lowercase** letters, an optional English possessive, optional closing
/// punctuation, and nothing else. So `(reflowing),` qualifies and none of
/// these do:
///
/// - `snake_case`, `main.rs`, `https://…`, `v2`, `a/b` — not a letter run.
/// - `README`, `JavaScript`, `Carrel` — any capital (in German, any capital
///   after the first letter). Acronyms and identifiers must not divide, and
///   a proper noun should not; the cost is that the first word of a
///   sentence does not either.
/// - `naïve`, `café` in English — the patterns cannot spell them, so their
///   answer for those words is not an answer. Likewise a letter outside the
///   language's own alphabet anywhere else, `ĳ` in Dutch included.
/// - `doesn’t` — a contraction is not a word followed by punctuation.
///   (French and Italian elision, `l’organisation`, is: their patterns
///   handle the apostrophe themselves.)
#[must_use]
pub(crate) fn word_in(language: Hyphenation, content: &str) -> Option<Range<usize>> {
    let p = language.patterns()?;
    let start = content.len() - content.trim_start_matches(OPENERS).len();
    let mut end = start;
    for (i, (at, c)) in content[start..].char_indices().enumerate() {
        if !p.letter(i, c) {
            break;
        }
        end = start + at + c.len_utf8();
    }
    // A closing apostrophe is punctuation, not elision.
    let end = start + content[start..end].trim_end_matches(is_apostrophe).len();
    let mut rest = &content[end..];
    if p.possessive {
        for possessive in ["'s", "\u{2019}s"] {
            if let Some(r) = rest.strip_prefix(possessive) {
                rest = r;
                break;
            }
        }
    }
    if end == start || !rest.trim_start_matches(CLOSERS).is_empty() {
        return None;
    }
    Some(start..end)
}

/// `word`'s letters, folded to lowercase, or `None` when it has too many to
/// divide. `word` is what [`word_in`] found.
fn letters(word: &str) -> Option<([char; MAX_WORD], usize)> {
    let mut out = ['.'; MAX_WORD];
    let mut n = 0;
    for c in word.chars() {
        *out.get_mut(n)? = lowercase(c).unwrap_or(c);
        n += 1;
    }
    Some((out, n))
}

/// Every place `word` may be divided, earliest first, as `(letters before
/// it, byte offset into word)`. `word` is what [`word_in`] found.
pub(crate) fn divisions(
    language: Hyphenation,
    word: &str,
) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mask = match (language.patterns(), letters(word)) {
        (Some(p), Some((chars, n))) => p.points(&chars[..n]),
        _ => 0,
    };
    word.char_indices()
        .enumerate()
        .filter(move |&(i, _)| i < 64 && mask & (1 << i) != 0)
        .map(|(i, (b, _))| (i, b))
}

/// Every byte offset at which `word` may be divided in `language`, earliest
/// first: what the layout chooses from. Empty for anything that is not a
/// whole divisible word of that language — punctuation included, so pass
/// the word alone.
#[must_use]
pub fn division_points(language: Hyphenation, word: &str) -> Vec<usize> {
    if word_in(language, word) != Some(0..word.len()) {
        return Vec::new();
    }
    divisions(language, word).map(|(_, b)| b).collect()
}

/// The latest division of `word` that leaves at most `max_head` letters
/// before the hyphen, as a byte offset into `word`.
#[must_use]
pub(crate) fn division(language: Hyphenation, word: &str, max_head: usize) -> Option<usize> {
    divisions(language, word)
        .take_while(|&(i, _)| i <= max_head)
        .last()
        .map(|(_, b)| b)
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

/// The language a document's frontmatter declares, as the hyphenation it
/// asks for — [`Hyphenation::Off`] for a language with no patterns here.
///
/// `lang:` or `language:` in a YAML block, `lang = "…"` in a TOML one. Not a
/// YAML parse — a reader must never fail on exotic YAML — so this reads the
/// one line it is looking for and nothing else. `None` when no line names a
/// language: the key is absent, or its value is empty.
fn declared(doc: &Document) -> Option<Hyphenation> {
    let node = doc.layout_order.first().map(|id| &doc.nodes[id.get()])?;
    let NodeKind::Metadata { format, .. } = node.kind else {
        return None;
    };
    let body = &doc.text[node.doc.start as usize..node.doc.end as usize];
    // Key lines through the core's one predicate, at the card's OWN
    // separator — what the card styles as a key is what is read as one.
    body.lines().find_map(|line| {
        let m = crate::document::meta_split(line, format)?;
        if !matches!(&line[..m.key_end], "lang" | "language") {
            return None;
        }
        Hyphenation::from_tag(&line[m.value..])
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

/// How this document's prose should be hyphenated: by the language its
/// frontmatter declares, else as English if its prose reads as English.
///
/// A declared language always wins, in both directions — `lang: de` gets
/// German patterns for a document that quotes a great deal of English, and
/// `lang: en` gets English ones for a document too short to judge. A
/// declared language with no patterns here (`sv`, `ja`, `xx`) is
/// [`Hyphenation::Off`], never English: hyphenating with the wrong
/// language's patterns is worse than not hyphenating.
///
/// Only English is ever inferred from the prose, and only because it was the
/// one language before there were others. A German page that does not say
/// so is not hyphenated at all.
#[must_use]
pub fn hyphenation_for(doc: &Document) -> Hyphenation {
    declared(doc).unwrap_or_else(|| {
        if reads_as_english(doc) {
            Hyphenation::English
        } else {
            Hyphenation::Off
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `word` with a `-` at every permitted division, in `language`.
    fn divided_in(language: Hyphenation, word: &str) -> String {
        let mut out = String::new();
        let mut from = 0;
        for (_, b) in divisions(language, word) {
            out.push_str(&word[from..b]);
            out.push('-');
            from = b;
        }
        out.push_str(&word[from..]);
        out
    }

    /// `word` with a `-` at every permitted English division.
    fn divided(word: &str) -> String {
        divided_in(Hyphenation::English, word)
    }

    /// The English division mask of `word`, which must be ASCII.
    fn points(word: &str) -> u64 {
        let chars: Vec<char> = word.chars().collect();
        if !chars.iter().all(char::is_ascii_lowercase) {
            return 0;
        }
        EN.points(&chars)
    }

    const LEFT_MIN: usize = 2;
    const RIGHT_MIN: usize = 3;

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
        let word = |s: &str| word_in(Hyphenation::English, s).map(|r| s[r].to_string());
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
        assert_eq!(division(Hyphenation::English, "hyphenation", 10), Some(6));
        assert_eq!(division(Hyphenation::English, "hyphenation", 6), Some(6));
        assert_eq!(division(Hyphenation::English, "hyphenation", 5), Some(2));
        assert_eq!(division(Hyphenation::English, "hyphenation", 1), None);
        assert_eq!(division(Hyphenation::English, "project", 6), None);
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
        assert_eq!(hyphenation_for(&Document::parse(&de)), Hyphenation::German);
        let sv = format!("---\nlang: sv\n---\n\n{ENGLISH_PROSE}");
        assert_eq!(hyphenation_for(&Document::parse(&sv)), Hyphenation::Off);
        // The card's own separator, through the core's one predicate: a
        // YAML key `lang=en` is not `lang`, and a TOML key `lang: x` is not
        // either — before, each was split at whichever character came first.
        let yaml_key = format!("---\nlang=en: x\n---\n\n{ENGLISH_PROSE}");
        assert_eq!(declared(&Document::parse(&yaml_key)), None);
        let toml_key = "+++\nlanguage: x = \"de\"\n+++\n\nShort.";
        assert_eq!(declared(&Document::parse(toml_key)), None);
        let toml = "+++\nlang=\"de\"\n+++\n\nShort.";
        assert_eq!(declared(&Document::parse(toml)), Some(Hyphenation::German));
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

    #[test]
    fn each_language_divides_its_own_words() {
        // Each is what TeX makes of the word. The whole set was checked
        // against an independent implementation (Typst's `hypher`) over
        // 61,000 Wikipedia words in these ten languages on 2026-10-19: the
        // two agree except on the exception lists, which `hypher` does not
        // apply, and on French elision with `’`, which it mishandles.
        use Hyphenation::*;
        for (language, word, want) in [
            (German, "geschichte", "ge-schich-te"),
            (German, "bevölkerung", "be-völ-ke-rung"),
            (French, "gouvernement", "gou-ver-ne-ment"),
            (French, "révolution", "ré-vo-lu-tion"),
            (Spanish, "constitución", "cons-ti-tu-ción"),
            (Italian, "costituzione", "co-sti-tu-zio-ne"),
            (Portuguese, "comissão", "co-mis-são"),
            (Dutch, "nederlandse", "ne-der-land-se"),
            (Danish, "befolkningen", "be-folk-nin-gen"),
            (Norwegian, "regjeringen", "re-gje-rin-gen"),
            (Polish, "województwo", "wo-je-wódz-two"),
        ] {
            assert_eq!(divided_in(language, word), want, "{language:?}");
        }
        // An exception list is per language, like the patterns.
        assert_eq!(divided_in(Dutch, "bestuurslagen"), "be-stuurs-la-gen");
        assert_eq!(divided_in(Polish, "przynajmniej"), "przy-naj-mniej");
    }

    #[test]
    fn a_language_tag_names_its_patterns_or_turns_them_off() {
        use Hyphenation::*;
        for (tag, want) in [
            ("de", German),
            ("de-DE", German),
            ("DE_ch", German),
            ("Deutsch", German),
            ("\"fr-CA\"", French),
            ("'es-419'", Spanish),
            ("pt-BR", Portuguese),
            ("nb-NO", Norwegian),
            ("nn", Norwegian),
            ("no", Norwegian),
            ("nld", Dutch),
            ("en-GB", English),
            // The traditional orthography divides `ck` and `st` differently.
            ("de-1901", Off),
            ("de-CH-1901", Off),
            // Patterns that exist but could not be embedded (LPPL-only),
            // languages with none, and nonsense: all off, never English.
            ("sv", Off),
            ("ru", Off),
            ("ja", Off),
            ("zh-Hans", Off),
            ("xx", Off),
            ("[en, de]", Off),
        ] {
            assert_eq!(Hyphenation::from_tag(tag), Some(want), "{tag:?}");
        }
        for nothing in ["", "  ", "\"\"", "''"] {
            assert_eq!(Hyphenation::from_tag(nothing), None, "{nothing:?}");
        }
    }

    #[test]
    fn a_declared_language_with_no_patterns_is_never_hyphenated_as_english() {
        for tag in ["xx", "sv", "ja", "de-1901"] {
            let src = format!("---\nlang: {tag}\n---\n\n{ENGLISH_PROSE}");
            assert_eq!(
                hyphenation_for(&Document::parse(&src)),
                Hyphenation::Off,
                "lang: {tag}"
            );
        }
        // An empty value declares nothing, so the prose decides.
        let src = format!("---\nlang:\n---\n\n{ENGLISH_PROSE}");
        assert_eq!(
            hyphenation_for(&Document::parse(&src)),
            Hyphenation::English
        );
        // And a German page that does not say so is not guessed at.
        let src = "Die Bundesrepublik Deutschland ist ein Bundesstaat in \
            Mitteleuropa. Ihre Hauptstadt ist Berlin, und die Bevölkerung \
            wächst seit der Wiedervereinigung nur langsam.";
        assert_eq!(hyphenation_for(&Document::parse(src)), Hyphenation::Off);
    }

    #[test]
    fn only_german_lets_a_word_open_with_a_capital() {
        use Hyphenation::*;
        let word = |l: Hyphenation, s: &str| word_in(l, s).map(|r| s[r].to_string());
        assert_eq!(
            word(German, "Wiedervereinigung,").as_deref(),
            Some("Wiedervereinigung")
        );
        assert_eq!(
            divided_in(German, "Wiedervereinigung"),
            "Wie-der-ver-ei-ni-gung"
        );
        assert_eq!(divided_in(German, "Straßenbahn"), "Stra-ßen-bahn");
        // Acronyms and inner capitals are still names, in German too.
        for not in ["BUNDESREPUBLIK", "DeutschLand", "iPhone", "Ünd3"] {
            assert_eq!(word(German, not), None, "{not:?}");
        }
        // Elsewhere a capital is a proper noun or a sentence's first word.
        assert_eq!(word(French, "Révolution"), None);
        assert_eq!(word(English, "Hyphenation"), None);
        assert_eq!(word(French, "révolution").as_deref(), Some("révolution"));
    }

    #[test]
    fn an_apostrophe_belongs_to_the_word_only_where_the_patterns_know_it() {
        use Hyphenation::*;
        let word = |l: Hyphenation, s: &str| word_in(l, s).map(|r| s[r].to_string());
        // French and Italian elision: the patterns forbid a division beside
        // the apostrophe themselves (`2’2`), and the elided article is part
        // of the word they score.
        assert_eq!(
            word(French, "l\u{2019}agriculture,").as_deref(),
            Some("l\u{2019}agriculture")
        );
        assert_eq!(
            divided_in(French, "l\u{2019}agriculture"),
            "l\u{2019}agri-cul-ture"
        );
        assert_eq!(divided_in(French, "aujourd'hui"), "au-jour-d'hui");
        assert_eq!(
            divided_in(Italian, "dell\u{2019}italia"),
            "del-l\u{2019}i-ta-lia"
        );
        // A closing apostrophe is a quotation mark, not elision.
        assert_eq!(
            word(French, "\u{2018}gouvernement\u{2019}").as_deref(),
            Some("gouvernement")
        );
        // Languages whose patterns have no apostrophe end the word at it.
        assert_eq!(word(German, "geht\u{2019}s"), None);
        assert_eq!(word(Dutch, "auto's"), None);
        assert_eq!(word(English, "doesn\u{2019}t"), None);
    }

    #[test]
    fn dutch_never_divides_the_ij() {
        // `ij` is one letter to a Dutch reader; the patterns know it.
        for word in [
            "bijvoorbeeld",
            "vrijheid",
            "rijkswaterstaat",
            "wijziging",
            "gemeentelijke",
            "verschijnsel",
            "eigenlijk",
            "bijzonderheden",
        ] {
            let divided = divided_in(Hyphenation::Dutch, word);
            assert!(divided.contains('-'), "{word}: the fixture must divide");
            assert!(!divided.contains("i-j"), "{divided}");
        }
        // The ligature is not in the patterns' alphabet, so a word spelled
        // with it is left whole rather than scored without it.
        assert_eq!(word_in(Hyphenation::Dutch, "bĳvoorbeeld"), None);
    }

    #[test]
    fn a_soft_hyphen_ends_a_word_for_the_patterns() {
        // The author already said where this compound divides; the patterns
        // must not divide its pieces again as if they were words.
        assert_eq!(
            word_in(Hyphenation::German, "Donau\u{AD}dampfschifffahrt"),
            None
        );
    }

    /// FNV-1a over a file's data lines, each followed by `\n`: stable across
    /// Rust versions, unlike `DefaultHasher`, so the pins below mean the same
    /// thing on every toolchain.
    fn fingerprint(source: &str) -> (usize, u64) {
        let (mut n, mut h) = (0, 0xcbf2_9ce4_8422_2325_u64);
        for line in data_lines(source) {
            n += 1;
            for b in line.bytes().chain(*b"\n") {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        (n, h)
    }

    #[test]
    fn the_embedded_data_is_hyph_utf8s_byte_for_byte() {
        // What the trie is built from, pinned per file. Each pin is the
        // fingerprint of hyph-utf8's own txt file at the commit the headers
        // name (tex-hyphen 5684c0f5), computed from upstream, not from these
        // files — so a header edit that leaks a line into the data, or a
        // header that swallows a pattern, fails here.
        const NONE: (usize, u64) = (0, 0xcbf2_9ce4_8422_2325);
        let pins = [
            (
                Hyphenation::English,
                (4938, 0xebec_970a_35db_5c81),
                (14, 0xd1d8_4bf0_7d24_f4ff),
            ),
            (Hyphenation::German, (36709, 0x12d4_cad9_22a8_34e7), NONE),
            (Hyphenation::French, (1216, 0x654a_853c_7dc2_cb5d), NONE),
            (Hyphenation::Spanish, (4694, 0x931d_bf2a_04f2_75ea), NONE),
            (Hyphenation::Italian, (384, 0xe552_3fb2_c60c_003a), NONE),
            (
                Hyphenation::Portuguese,
                (427, 0xaddc_2ae3_54da_1924),
                (2, 0x5436_7533_2796_d44c),
            ),
            (
                Hyphenation::Dutch,
                (12724, 0x79d1_43c4_be76_4841),
                (40, 0x159c_2fda_fe13_f6a7),
            ),
            (Hyphenation::Danish, (1144, 0xcaa3_b12e_de2a_e096), NONE),
            (
                Hyphenation::Norwegian,
                (27448, 0x9f14_d9dd_e0fa_5e41),
                (2, 0x7be0_602b_8ca5_ce25),
            ),
            (
                Hyphenation::Polish,
                (4053, 0x9c5f_09f8_9b86_a795),
                (20, 0x8238_496b_9500_95a6),
            ),
        ];
        for (language, pat, hyp) in pins {
            let p = language.patterns().expect("has patterns");
            assert_eq!(fingerprint(p.pat), pat, "{language:?} patterns");
            assert_eq!(fingerprint(p.hyp), hyp, "{language:?} exceptions");
        }
        let pinned: Vec<_> = pins.iter().map(|&(l, ..)| l).collect();
        assert_eq!(
            pinned,
            Hyphenation::LANGUAGES,
            "pin every language, in order"
        );
    }

    #[test]
    fn every_data_line_is_pattern_or_exception_syntax() {
        // A TeX command in the data is not a comment to the parser: it is a
        // pattern, and its characters join the language's alphabet. The
        // German file once carried `\message{…}` that way, which put `'` in
        // German's alphabet and so made `geht's` one divisible word.
        for language in Hyphenation::LANGUAGES {
            let p = language.patterns().expect("has patterns");
            for line in data_lines(p.pat) {
                assert!(
                    line.chars().all(|c| c.is_alphabetic()
                        || c.is_ascii_digit()
                        || matches!(c, '.' | '-' | '\'' | '\u{2019}')),
                    "{language:?}: {line:?} is not a pattern"
                );
            }
            for word in data_lines(p.hyp).flat_map(str::split_whitespace) {
                assert!(
                    word.chars().all(|c| c.is_alphabetic() || c == '-'),
                    "{language:?}: {word:?} is not an exception"
                );
            }
        }
        assert_eq!(word_in(Hyphenation::German, "geht's"), None);
    }

    #[test]
    fn every_embedded_file_carries_its_licence() {
        // Each id's licence text, as the file must carry it. The SPDX line is
        // carrel's own; the text is upstream's, except Dutch, whose header
        // names MIT by URL alone (see that file).
        let texts = [
            ("MIT", "Permission is hereby granted, free of charge"),
            (
                "BSD-3-Clause",
                "Redistributions in binary form must reproduce",
            ),
            (
                "FSFAP-no-warranty-disclaimer",
                "Copying and distribution of this file, with or without modification,",
            ),
            (
                "Knuth-CTAN",
                "Unlimited copying and redistribution of this file",
            ),
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hyphen");
        let notices = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("THIRD-PARTY-NOTICES"),
        )
        .expect("THIRD-PARTY-NOTICES ships with carrel-core");
        let embedded: Vec<&str> = Hyphenation::LANGUAGES
            .iter()
            .flat_map(|l| {
                let p = l.patterns().expect("has patterns");
                [p.pat, p.hyp]
            })
            .collect();
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("src/hyphen") {
            let path = entry.expect("entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.starts_with("hyph-") {
                continue;
            }
            seen += 1;
            let body = std::fs::read_to_string(&path).expect("utf-8");
            assert!(
                embedded.contains(&body.as_str()),
                "{name} is in src/hyphen but no language embeds it"
            );
            assert!(body.starts_with('%'), "{name}: the notice comes first");
            let header: Vec<&str> = body.lines().take_while(|l| l.starts_with('%')).collect();
            let header = header.join("\n");
            let ids = header
                .lines()
                .find_map(|l| l.strip_prefix("% SPDX-License-Identifier: "))
                .unwrap_or_else(|| panic!("{name}: no SPDX-License-Identifier line"));
            for id in ids.split(" AND ") {
                let (_, text) = texts
                    .iter()
                    .find(|(known, _)| *known == id)
                    .unwrap_or_else(|| panic!("{name}: {id} is not a licence carrel accepts"));
                assert!(
                    header.contains(text),
                    "{name}: says {id} but its header lacks the licence text"
                );
            }
            assert!(header.contains("Copyright"), "{name}: no copyright line");
            assert!(
                notices.contains(&format!("src/hyphen/{name}")),
                "{name} is not listed in THIRD-PARTY-NOTICES"
            );
        }
        assert_eq!(seen, 15, "every hyph-* file was checked");
    }

    #[test]
    fn words_of_no_alphabet_here_never_divide() {
        // CJK has no patterns, and English patterns cannot spell it.
        for language in Hyphenation::LANGUAGES {
            assert_eq!(word_in(language, "日本語のテキスト"), None);
            assert_eq!(word_in(language, "документация"), None, "{language:?}");
        }
    }
}
