//! Fuzzy subsequence scoring for the pickers — fzf's idea, hand-rolled to
//! the size this project will carry.
//!
//! A needle matches when every one of its characters appears in order,
//! case-insensitively. The score prefers what a reader means: matches that
//! start at a boundary (`/`, `_`, `-`, `.`, a space, a camel hump), runs of
//! consecutive characters, and needles found tightly rather than scattered.
//! A tiny dynamic program finds the BEST alignment rather than the first,
//! which is what makes `docs/readme.md` beat `r/m/d/notes.md` for `rmd` —
//! a greedy left-to-right scan cannot see that trade-off.
//!
//! Two rows of `O(m)` cells, rebuilt per needle character: cheap enough to
//! rank a folder of 100,000 notes on every keystroke.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

/// Rewarded when a match lands at a word-like edge.
const BOUNDARY: i32 = 16;
/// Rewarded for each adjacent pair inside a matched run.
const CONSECUTIVE: i32 = 8;
/// Charged per character skipped between two matches.
const GAP: i32 = 1;
/// Sentinel floor: anything at or below this is "no alignment".
const FLOOR: i32 = i32::MIN / 4;

/// Positional bonus for matching at haystack index `i`.
fn bonus_at(hay: &[char], i: usize) -> i32 {
    if i == 0 {
        return BOUNDARY;
    }
    let prev = hay[i - 1];
    let here = hay[i];
    if matches!(prev, '/' | '_' | '-' | '.' | ' ' | '#') {
        BOUNDARY
    } else if prev.is_lowercase() && here.is_uppercase() {
        // A camel hump is a boundary too, just a quieter one.
        BOUNDARY / 2
    } else {
        0
    }
}

/// Score `needle` against `haystack`: `None` when it is not an in-order
/// case-insensitive subsequence, else a score where bigger is better.
#[must_use]
pub fn score(haystack: &str, needle: &str) -> Option<i32> {
    let hay: Vec<char> = haystack.chars().collect();
    let low: Vec<char> = hay
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let ned: Vec<char> = needle
        .chars()
        .filter_map(|c| c.to_lowercase().next())
        .collect();
    let (n, m) = (ned.len(), low.len());
    if n == 0 {
        return Some(0);
    }
    if n > m {
        return None;
    }
    // Cheap rejection before any table work: it must be a subsequence at all.
    let mut seen = 0usize;
    for &c in &low {
        if seen < n && c == ned[seen] {
            seen += 1;
        }
    }
    if seen < n {
        return None;
    }

    // `prev[i]`: best score of an alignment of needle[..j] whose LAST match
    // sits exactly on haystack index i. `FLOOR` means impossible there.
    let mut prev = vec![FLOOR; m];
    let mut cur = vec![FLOOR; m];
    for (j, nc) in ned.iter().enumerate() {
        // Running max over k <= i-1 of `prev[k] + GAP*k`, so the inner-gap
        // cost `GAP*(i-1-k)` collapses into one lookup — the trick that
        // keeps this O(n*m) instead of cubed.
        let mut run = FLOOR;
        for i in 0..m {
            cur[i] = FLOOR;
            if low[i] == *nc {
                let base = bonus_at(&hay, i);
                if j == 0 {
                    cur[i] = base;
                } else if i > 0 {
                    // Arrive from any earlier match k<i, paying one GAP per
                    // character skipped between k and i…
                    let prev_i = i32::try_from(i - 1).unwrap_or(i32::MAX);
                    let spread = if run > FLOOR {
                        run.checked_sub(prev_i * GAP).unwrap_or(FLOOR)
                    } else {
                        FLOOR
                    };
                    // …or extend a run: needle[j-1] matched exactly at i-1.
                    let adjacent = if prev[i - 1] > FLOOR {
                        prev[i - 1].checked_add(CONSECUTIVE).unwrap_or(FLOOR)
                    } else {
                        FLOOR
                    };
                    let via = spread.max(adjacent);
                    cur[i] = if via > FLOOR {
                        base.saturating_add(via)
                    } else {
                        FLOOR
                    };
                }
            }
            let ii = i32::try_from(i).unwrap_or(i32::MAX);
            let r = prev[i].checked_add(ii * GAP).unwrap_or(FLOOR);
            run = run.max(r);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.into_iter().max().filter(|&v| v > FLOOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subsequence_matches_case_insensitively() {
        assert!(score("README.md", "rmd").is_some());
        assert_eq!(score("abc", ""), Some(0), "empty needle matches all");
    }

    #[test]
    fn out_of_order_or_missing_letters_reject() {
        assert_eq!(score("abc", "ca"), None);
        assert_eq!(score("short", "longer than it"), None);
    }

    #[test]
    fn consecutive_runs_beat_scattered_ones() {
        let tight = score("abcd", "ab").expect("match");
        let loose = score("axbycd", "ab").expect("match");
        assert!(tight > loose);
    }

    #[test]
    fn tighter_gaps_beat_longer_ones_at_equal_boundaries() {
        let near = score("abxd", "abd").expect("match");
        let far = score("abxxxd", "abd").expect("match");
        assert!(near > far, "{near} vs {far}");
    }

    #[test]
    fn boundaries_beat_word_interiors() {
        let path = score("notes/readme.md", "rea").expect("match");
        let interior = score("xarea.md", "rea").expect("match");
        assert!(path > interior, "{path} vs {interior}");
    }

    #[test]
    fn a_camel_hump_is_a_quieter_boundary() {
        let hump = score("myDoc.md", "d").expect("match");
        let flat = score("mydoc.md", "d").expect("match");
        assert!(hump > flat, "{hump} vs {flat}");
    }

    #[test]
    fn the_best_alignment_wins_not_the_first() {
        // Two a's: the LEFTMOST one strands the b (every pairing through it
        // scores below zero); skipping it buys an adjacent pair. 8 per
        // adjacency is the whole of an interior run — there is no per-char
        // base, so `Some(8)` IS the adjacent pair and nothing else.
        assert_eq!(score("xaabbx", "ab"), Some(8));
    }
}
