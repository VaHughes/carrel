//! Bounded word comparison of adjacent removal/addition runs. The output is
//! still sorted, non-overlapping doc-byte tokens; text is never rewritten.
use super::{Token, TokenKind};
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn emphasize(text: &str, base: u32, tokens: Vec<Token>) -> Vec<Token> {
    let mut lines = Vec::new();
    let mut offset = 0;
    // Lines the hunk being read still has to come, old side and new. In a
    // hunk a line is what its sigil says, whatever follows the sigil: a
    // removed `-- note` is written `--- note`.
    let mut hunk = (0usize, 0usize);
    let mut misread: Vec<Token> = Vec::new();
    for line in text.split_inclusive('\n') {
        let in_hunk = hunk != (0, 0);
        let first = line.as_bytes().first();
        let side = match first {
            Some(b'-') if in_hunk || !line.starts_with("--- ") => Side::Old,
            Some(b'+') if in_hunk || !line.starts_with("+++ ") => Side::New,
            _ => Side::Neither,
        };
        if in_hunk {
            let both = !matches!(first, Some(b'-' | b'+' | b'\\'));
            hunk.0 = hunk
                .0
                .saturating_sub(usize::from(side == Side::Old || both));
            hunk.1 = hunk
                .1
                .saturating_sub(usize::from(side == Side::New || both));
        } else if let Some(counts) = crate::diff::hunk_lines(line.trim_end()) {
            hunk = counts;
        }
        // The grammar reads such a line as a file header whatever hunk it
        // is in, and it was painted as one: the color of plumbing.
        if in_hunk && (line.starts_with("---") || line.starts_with("+++")) {
            let kind = if side == Side::Old {
                TokenKind::Deleted
            } else {
                TokenKind::Inserted
            };
            let at = base + offset as u32;
            misread.push(Token {
                doc: at..at + line.len() as u32,
                kind,
            });
        }
        // This marker describes the preceding line; it is not context and
        // must not split a replacement at end of file.
        if !line.starts_with("\\ No newline at end of file") {
            lines.push((offset, line, side));
        }
        offset += line.len();
    }
    let mut changed = Vec::new();
    let mut i = 0;
    // A total budget, not a per-line budget: adversarial fences stay cheap.
    let mut budget = 1_000_000usize;
    while i < lines.len() {
        let start = i;
        while i < lines.len() && lines[i].2 == Side::Old {
            i += 1;
        }
        let middle = i;
        while i < lines.len() && lines[i].2 == Side::New {
            i += 1;
        }
        if start < middle && middle < i {
            let old = words(&lines[start..middle]);
            let new = words(&lines[middle..i]);
            let cells = (old.len() + 1).saturating_mul(new.len() + 1);
            if cells <= budget {
                budget -= cells;
                compare(&old, &new, &mut changed);
            }
        }
        if i == start {
            i += 1;
        }
    }
    changed.sort_unstable_by_key(|r| r.start);
    let tokens = reclassified(tokens, misread);
    let mut result = Vec::new();
    let mut index = 0;
    for token in tokens {
        let mut at = token.doc.start;
        while index < changed.len() && base + changed[index].end <= at {
            index += 1;
        }
        let mut j = index;
        while j < changed.len() && base + changed[j].start < token.doc.end {
            let start = (base + changed[j].start).max(at);
            let end = (base + changed[j].end).min(token.doc.end);
            if start > at {
                result.push(Token {
                    doc: at..start,
                    kind: token.kind,
                });
            }
            let kind = match token.kind {
                TokenKind::Inserted => TokenKind::InsertedWord,
                TokenKind::Deleted => TokenKind::DeletedWord,
                other => other,
            };
            if start < end {
                result.push(Token {
                    doc: start..end,
                    kind,
                });
            }
            at = end;
            j += 1;
        }
        if at < token.doc.end {
            result.push(Token {
                doc: at..token.doc.end,
                kind: token.kind,
            });
        }
    }
    result
}

/// `tokens` with each of `lines` — whole lines, in order — in place of
/// whatever the grammar made of that line.
fn reclassified(tokens: Vec<Token>, lines: Vec<Token>) -> Vec<Token> {
    if lines.is_empty() {
        return tokens;
    }
    let mut out = Vec::with_capacity(tokens.len() + lines.len());
    for token in tokens {
        let mut at = token.doc.start;
        let mut next = lines.partition_point(|line| line.doc.end <= at);
        while at < token.doc.end {
            match lines.get(next) {
                Some(line) if line.doc.start < token.doc.end => {
                    if line.doc.start > at {
                        out.push(Token {
                            doc: at..line.doc.start,
                            kind: token.kind,
                        });
                    }
                    at = line.doc.end;
                    next += 1;
                }
                _ => {
                    out.push(Token {
                        doc: at..token.doc.end,
                        kind: token.kind,
                    });
                    break;
                }
            }
        }
    }
    out.extend(lines);
    out.sort_unstable_by_key(|token| token.doc.start);
    out
}

/// Which side of a change a line belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Old,
    New,
    Neither,
}

type Word<'a> = (u32, &'a str);
fn words<'a>(lines: &[(usize, &'a str, Side)]) -> Vec<Word<'a>> {
    lines
        .iter()
        .flat_map(|(offset, line, _)| {
            line[1..]
                .split_word_bound_indices()
                .filter(|(_, word)| !word.chars().all(char::is_whitespace))
                .map(move |(at, word)| ((offset + 1 + at) as u32, word))
        })
        .collect()
}

fn compare(old: &[Word<'_>], new: &[Word<'_>], changed: &mut Vec<std::ops::Range<u32>>) {
    let stride = new.len() + 1;
    let mut lcs = vec![0u32; (old.len() + 1) * stride];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i * stride + j] = if old[i].1 == new[j].1 {
                1 + lcs[(i + 1) * stride + j + 1]
            } else {
                lcs[(i + 1) * stride + j].max(lcs[i * stride + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i].1 == new[j].1 {
            i += 1;
            j += 1;
        } else if i < old.len()
            && (j == new.len() || lcs[(i + 1) * stride + j] >= lcs[i * stride + j + 1])
        {
            changed.push(old[i].0..old[i].0 + old[i].1.len() as u32);
            i += 1;
        } else {
            changed.push(new[j].0..new[j].0 + new[j].1.len() as u32);
            j += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::highlight;

    fn changes(text: &str) -> Vec<(String, TokenKind)> {
        let tokens = highlight("diff", text, 137);
        assert!(tokens.windows(2).all(|w| w[0].doc.end <= w[1].doc.start));
        tokens
            .into_iter()
            .filter(|t| matches!(t.kind, TokenKind::InsertedWord | TokenKind::DeletedWord))
            .map(|t| {
                (
                    text[(t.doc.start - 137) as usize..(t.doc.end - 137) as usize].to_owned(),
                    t.kind,
                )
            })
            .collect()
    }

    #[test]
    fn emphasizes_changed_words_and_punctuation_not_shared_prose() {
        assert_eq!(
            changes("@@ -1 +1 @@\n-The quiet reader works.\n+The quick reader works!\n"),
            vec![
                ("quiet".into(), TokenKind::DeletedWord),
                (".".into(), TokenKind::DeletedWord),
                ("quick".into(), TokenKind::InsertedWord),
                ("!".into(), TokenKind::InsertedWord),
            ]
        );
    }

    #[test]
    fn unicode_and_multiline_replacements_keep_byte_offsets() {
        let result = changes("-A café serves\n-old tea.\n+A café\n+serves fresh tea.\n");
        assert_eq!(
            result,
            vec![
                ("old".into(), TokenKind::DeletedWord),
                ("fresh".into(), TokenKind::InsertedWord)
            ]
        );
        assert_eq!(
            changes("-café 👩‍💻\n+thé 👩‍💻\n"),
            vec![
                ("café".into(), TokenKind::DeletedWord),
                ("thé".into(), TokenKind::InsertedWord)
            ]
        );
    }

    #[test]
    fn headers_context_and_unpaired_changes_are_not_word_diffs() {
        assert_eq!(
            changes("--- a/old\n+++ b/new\n@@ -1 +1 @@\n-only removed\n context\n+only added\n"),
            [] as [(String, crate::highlight::TokenKind); 0]
        );
        assert_eq!(
            changes("-same words\n+same   words\n"),
            [] as [(String, crate::highlight::TokenKind); 0]
        );
        assert_eq!(
            changes("-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"),
            vec![
                ("old".into(), TokenKind::DeletedWord),
                ("new".into(), TokenKind::InsertedWord)
            ]
        );
    }

    /// Inside a hunk a line is what its sigil says: a removed `-- note` is
    /// written `--- note`, and was passed over as a file header.
    #[test]
    fn a_hunk_line_that_looks_like_a_header_is_still_compared() {
        assert_eq!(
            changes("@@ -1,2 +1,2 @@\n context\n--- old note\n+-- new note\n"),
            vec![
                ("old".into(), TokenKind::DeletedWord),
                ("new".into(), TokenKind::InsertedWord)
            ]
        );
        assert_eq!(
            changes("@@ -1 +1 @@\n-- i = 1;\n+++ i = 2;\n"),
            vec![
                ("-".into(), TokenKind::DeletedWord),
                ("1".into(), TokenKind::DeletedWord),
                ("+".into(), TokenKind::InsertedWord),
                ("+".into(), TokenKind::InsertedWord),
                ("2".into(), TokenKind::InsertedWord)
            ]
        );
        // And it is painted as the removal it is, not as a file header.
        let text = "@@ -1,2 +1,2 @@\n context\n--- old note\n+-- new note\n";
        let line = text.find("--- old").unwrap() as u32;
        let kinds: Vec<TokenKind> = highlight("diff", text, 0)
            .into_iter()
            .filter(|t| t.doc.start < line + 12 && t.doc.end > line)
            .map(|t| t.kind)
            .collect();
        assert!(
            !kinds.is_empty()
                && kinds
                    .iter()
                    .all(|k| matches!(k, TokenKind::Deleted | TokenKind::DeletedWord)),
            "{kinds:?}"
        );
        // After the hunk's lines, a header pair is a header pair again.
        assert_eq!(
            changes("@@ -1 +1 @@\n-a\n+a\n--- a/old\n+++ b/new\n"),
            [] as [(String, crate::highlight::TokenKind); 0]
        );
    }

    #[test]
    fn comparison_budget_falls_back_to_line_colors() {
        let text = format!("-{}\n+{}\n", "old ".repeat(1100), "new ".repeat(1100));
        assert_eq!(
            changes(&text),
            [] as [(String, crate::highlight::TokenKind); 0]
        );
        assert!(
            highlight("diff", &text, 0)
                .iter()
                .any(|t| t.kind == TokenKind::Inserted)
        );
    }
}
