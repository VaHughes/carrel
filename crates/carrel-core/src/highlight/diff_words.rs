//! Bounded word comparison of adjacent removal/addition runs. The output is
//! still sorted, non-overlapping doc-byte tokens; text is never rewritten.
use super::{Token, TokenKind};
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn emphasize(text: &str, base: u32, tokens: Vec<Token>) -> Vec<Token> {
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        // This marker describes the preceding line; it is not context and
        // must not split a replacement at end of file.
        if !line.starts_with("\\ No newline at end of file") {
            lines.push((offset, line));
        }
        offset += line.len();
    }
    let mut changed = Vec::new();
    let mut i = 0;
    // A total budget, not a per-line budget: adversarial fences stay cheap.
    let mut budget = 1_000_000usize;
    while i < lines.len() {
        let start = i;
        while i < lines.len() && removed(lines[i].1) {
            i += 1;
        }
        let middle = i;
        while i < lines.len() && added(lines[i].1) {
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

fn removed(s: &str) -> bool {
    s.starts_with('-') && !s.starts_with("--- ")
}
fn added(s: &str) -> bool {
    s.starts_with('+') && !s.starts_with("+++ ")
}

type Word<'a> = (u32, &'a str);
fn words<'a>(lines: &[(usize, &'a str)]) -> Vec<Word<'a>> {
    lines
        .iter()
        .flat_map(|(offset, line)| {
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
        assert!(
            changes("--- a/old\n+++ b/new\n@@ -1 +1 @@\n-only removed\n context\n+only added\n")
                .is_empty()
        );
        assert!(changes("-same words\n+same   words\n").is_empty());
        assert_eq!(
            changes("-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"),
            vec![
                ("old".into(), TokenKind::DeletedWord),
                ("new".into(), TokenKind::InsertedWord)
            ]
        );
    }

    #[test]
    fn comparison_budget_falls_back_to_line_colors() {
        let text = format!("-{}\n+{}\n", "old ".repeat(1100), "new ".repeat(1100));
        assert!(changes(&text).is_empty());
        assert!(
            highlight("diff", &text, 0)
                .iter()
                .any(|t| t.kind == TokenKind::Inserted)
        );
    }
}
