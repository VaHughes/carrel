//! What the command line meant, where that takes more than reading it.
//!
//! The binary stays thin: it matches on arguments and calls in here for the
//! three things that need judgment — an operand that names a place inside a
//! file, a typo that has an obvious correction, and "the newest document".
//! They live in the library so they can be tested without a process.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::path::{Path, PathBuf};

/// Where in a document to open it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// A 1-based line of the file, as `grep -n`, a compiler and an agent
    /// all print it.
    Line(u32),
    /// A heading's `#fragment`.
    Fragment(String),
}

/// An operand as the file it names and, optionally, the place in it.
///
/// An agent that has just written a plan says where to look the way every
/// tool does — `PLAN.md:42`, `notes.md:42:7`, `README.md#install` — and that
/// text gets pasted here. So:
///
/// - **A path that exists is that path.** A file really called `a:42` opens
///   as itself; nothing is split off a name that is already an answer.
/// - Otherwise a trailing `:LINE` or `:LINE:COL` is a line (the column is
///   accepted and ignored — a reader has rows, not a cursor), if what is
///   left names a file.
/// - Otherwise a `#fragment` is a section, if what is left names a file.
///   `#L42` is a line: GitHub's spelling of the same thing.
/// - Anything else is returned whole, for the ordinary "no such file".
#[must_use]
pub fn split_target(arg: &str) -> (PathBuf, Option<Start>) {
    let whole = || (PathBuf::from(arg), None);
    if Path::new(arg).exists() {
        return whole();
    }
    if let Some((path, line)) = line_suffix(arg)
        && Path::new(path).is_file()
    {
        return (PathBuf::from(path), Some(Start::Line(line)));
    }
    if let Some((path, frag)) = arg.rsplit_once('#')
        && !frag.is_empty()
        && Path::new(path).is_file()
    {
        let start = match frag.strip_prefix('L').and_then(|n| n.parse::<u32>().ok()) {
            Some(line) if line >= 1 => Start::Line(line),
            _ => Start::Fragment(frag.to_string()),
        };
        return (PathBuf::from(path), Some(start));
    }
    whole()
}

/// `path:LINE` or `path:LINE:COL`, as `(path, line)`.
fn line_suffix(arg: &str) -> Option<(&str, u32)> {
    let number = |s: &str| {
        (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
            .filter(|&n| n >= 1)
    };
    let (head, last) = arg.rsplit_once(':')?;
    let last = number(last)?;
    // Two numbers: the first is the line and the second a column.
    if let Some((path, line)) = head.rsplit_once(':')
        && let Some(line) = number(line)
    {
        return Some((path, line));
    }
    Some((head, last))
}

/// A path as it arrives when it is pasted or dropped onto the window.
///
/// Terminals and file managers each have their own idea of how to hand over
/// a file: bare, in quotes, with its spaces backslashed, or as a `file://`
/// URL with them percent-encoded. This undoes each of those and nothing
/// else. The first line only — a paste with several is not a path — and
/// `None` for anything that is not plausibly one, so a paragraph pasted by
/// accident does not become a "no such file" about its own first sentence.
#[must_use]
pub fn pasted_path(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.len() > 4096 || line.chars().any(char::is_control) {
        return None;
    }
    let quoted = line.len() >= 2
        && ((line.starts_with('"') && line.ends_with('"'))
            || (line.starts_with('\'') && line.ends_with('\'')));
    let mut path = if quoted {
        line[1..line.len() - 1].to_string()
    } else if let Some(rest) = line.strip_prefix("file://") {
        // `file:///home/x` and `file://localhost/home/x` both mean `/home/x`.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        crate::links::percent_decoded(rest).unwrap_or_else(|| rest.to_string())
    } else {
        // A shell-escaped drop: `/my\ notes/a.md`.
        let mut out = String::with_capacity(line.len());
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.extend(chars.next()),
                c => out.push(c),
            }
        }
        out
    };
    if path.is_empty() {
        return None;
    }
    if path == "~" || path.starts_with("~/") {
        path = crate::home::expand_typed(&path)
            .to_string_lossy()
            .into_owned();
    }
    // Prose has spaces and no separators; a path has a separator, or is a
    // single name with an extension.
    let looks_like_a_path = path.contains('/')
        || (!path.contains(' ') && path.contains('.'))
        || Path::new(&path).exists();
    looks_like_a_path.then_some(path)
}

/// How many single-character edits separate two strings, counting a swap of
/// neighbors as one — the typo people actually make (`--plian`).
///
/// Optimal string alignment distance, over `char`s.
#[must_use]
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev2: Vec<usize> = Vec::new();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (prev[j] + 1).min(row[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(prev2[j - 2] + 1);
            }
        }
        prev2 = std::mem::replace(&mut prev, row);
    }
    prev[b.len()]
}

/// How far apart a typo and its correction may be. Two edits, and never as
/// many as half the word: `-x` is not a misspelling of `-h`.
fn close_enough(typed: &str, candidate: &str) -> bool {
    let d = distance(typed, candidate);
    d > 0 && d <= 2 && d * 2 < typed.chars().count().max(candidate.chars().count())
}

/// The known option a mistyped one most plausibly meant.
#[must_use]
pub fn suggest_flag<'a>(typed: &str, known: &[&'a str]) -> Option<&'a str> {
    known
        .iter()
        .copied()
        .filter(|k| k.starts_with("--") && close_enough(typed, k))
        .min_by_key(|k| distance(typed, k))
}

/// The file beside a missing one that the name most plausibly meant.
///
/// Looks only in the directory the missing path names, and only at names:
/// the same name in another case, the same name with or without a markdown
/// extension, or a name one or two edits away. One answer or none — when two
/// files are equally near, guessing between them is worse than saying
/// nothing.
#[must_use]
pub fn suggest_file(missing: &Path) -> Option<PathBuf> {
    let name = missing.file_name()?.to_str()?;
    let dir = match missing.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let lower = name.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    let mut tied = false;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(candidate) = entry.file_name().into_string() else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let cl = candidate.to_lowercase();
        let score = if cl == lower {
            0
        } else if cl == format!("{lower}.md") || cl == format!("{lower}.markdown") {
            1
        } else if close_enough(&lower, &cl) {
            1 + distance(&lower, &cl)
        } else {
            continue;
        };
        match &best {
            Some((s, _)) if *s < score => {}
            Some((s, _)) if *s == score => tied = true,
            _ => {
                best = Some((score, candidate));
                tied = false;
            }
        }
    }
    let (_, found) = best.filter(|_| !tied)?;
    Some(match missing.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(found),
        _ => PathBuf::from(found),
    })
}

/// The most recently modified of `entries`: what `carrel --latest` opens.
///
/// Ties go to the earlier path, so the answer does not depend on the order
/// the walk happened to find them in.
#[must_use]
pub fn latest(entries: &[crate::scan::Entry]) -> Option<&Path> {
    entries
        .iter()
        .max_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| b.path.cmp(&a.path)))
        .map(|e| e.path.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "x\n").unwrap();
        p
    }

    #[test]
    fn a_line_or_a_section_is_split_off_a_file_that_exists() {
        let d = tempfile::tempdir().unwrap();
        let f = touch(d.path(), "PLAN.md");
        let s = f.to_str().unwrap();
        assert_eq!(
            split_target(&format!("{s}:42")),
            (f.clone(), Some(Start::Line(42)))
        );
        assert_eq!(
            split_target(&format!("{s}:42:7")),
            (f.clone(), Some(Start::Line(42))),
            "a column is accepted and ignored"
        );
        assert_eq!(
            split_target(&format!("{s}#step-3")),
            (f.clone(), Some(Start::Fragment("step-3".into())))
        );
        assert_eq!(
            split_target(&format!("{s}#L12")),
            (f.clone(), Some(Start::Line(12))),
            "GitHub's spelling of a line"
        );
        assert_eq!(split_target(s), (f, None));
    }

    #[test]
    fn a_name_that_exists_is_never_split() {
        // Files really are called these things, and the literal name wins.
        let d = tempfile::tempdir().unwrap();
        for name in ["notes:42", "a#b", "report:2026:09"] {
            let f = touch(d.path(), name);
            assert_eq!(split_target(f.to_str().unwrap()), (f, None), "{name}");
        }
    }

    #[test]
    fn what_is_not_a_place_in_a_file_is_left_whole() {
        let d = tempfile::tempdir().unwrap();
        let f = touch(d.path(), "PLAN.md");
        let s = f.to_str().unwrap();
        for arg in [
            format!("{s}:"),
            format!("{s}:0"),
            format!("{s}:abc"),
            format!("{s}:-3"),
            format!("{s}#"),
            format!("{s}:99999999999999999999"),
            format!("{}/missing.md:42", d.path().display()),
            format!("{}:42", d.path().display()),
        ] {
            assert_eq!(split_target(&arg), (PathBuf::from(&arg), None), "{arg}");
        }
    }

    #[test]
    fn a_pasted_path_is_read_however_it_was_handed_over() {
        let p = |s: &str| pasted_path(s);
        assert_eq!(p("/w/notes/PLAN.md").as_deref(), Some("/w/notes/PLAN.md"));
        assert_eq!(p("  /w/PLAN.md \n").as_deref(), Some("/w/PLAN.md"));
        assert_eq!(p("'/w/my notes/a.md'").as_deref(), Some("/w/my notes/a.md"));
        assert_eq!(
            p("\"/w/my notes/a.md\"").as_deref(),
            Some("/w/my notes/a.md")
        );
        assert_eq!(p("/w/my\\ notes/a.md").as_deref(), Some("/w/my notes/a.md"));
        assert_eq!(
            p("file:///w/my%20notes/a.md").as_deref(),
            Some("/w/my notes/a.md")
        );
        assert_eq!(p("file://localhost/w/a.md").as_deref(), Some("/w/a.md"));
        assert_eq!(
            p("PLAN.md:42").as_deref(),
            Some("PLAN.md:42"),
            "a place survives"
        );
        assert_eq!(
            p("docs/PLAN.md\nsecond line").as_deref(),
            Some("docs/PLAN.md")
        );
    }

    #[test]
    fn a_paste_that_is_not_a_path_is_not_treated_as_one() {
        for not in [
            "",
            "   \n  ",
            "This is a sentence someone copied by mistake.",
            "hello",
            "a\u{7}b.md",
        ] {
            assert_eq!(pasted_path(not), None, "{not:?}");
        }
        assert_eq!(pasted_path(&"x/".repeat(5000)), None, "absurdly long");
    }

    #[test]
    fn a_swap_of_neighbors_is_one_edit() {
        assert_eq!(distance("--plian", "--plain"), 1);
        assert_eq!(distance("--plain", "--plain"), 0);
        assert_eq!(distance("--tutoral", "--tutorial"), 1);
        assert_eq!(distance("abc", ""), 3);
        assert_eq!(distance("PLAM.md", "PLAN.md"), 1);
    }

    #[test]
    fn a_mistyped_option_suggests_the_one_it_is_near() {
        let known = ["-h", "--help", "--plain", "--render", "--tasks", "--latest"];
        assert_eq!(suggest_flag("--plian", &known), Some("--plain"));
        assert_eq!(suggest_flag("--rendr", &known), Some("--render"));
        assert_eq!(suggest_flag("--lates", &known), Some("--latest"));
        assert_eq!(suggest_flag("--verbose", &known), None, "nothing is near");
        assert_eq!(
            suggest_flag("-x", &known),
            None,
            "a short flag is not a typo"
        );
        assert_eq!(
            suggest_flag("--plain", &known),
            None,
            "it is not a typo at all"
        );
    }

    #[test]
    fn a_missing_file_suggests_its_neighbor() {
        let d = tempfile::tempdir().unwrap();
        let plan = touch(d.path(), "PLAN.md");
        touch(d.path(), "CHANGELOG.md");
        touch(d.path(), "README.md");
        assert_eq!(suggest_file(&d.path().join("PLAM.md")), Some(plan.clone()));
        assert_eq!(suggest_file(&d.path().join("plan.md")), Some(plan.clone()));
        assert_eq!(suggest_file(&d.path().join("PLAN")), Some(plan));
        assert_eq!(
            suggest_file(&d.path().join("readme")),
            Some(d.path().join("README.md"))
        );
        assert_eq!(suggest_file(&d.path().join("budget.md")), None);
        assert_eq!(suggest_file(&d.path().join("nowhere/PLAN.md")), None);
    }

    #[test]
    fn two_equally_near_files_are_not_guessed_between() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "note1.md");
        touch(d.path(), "note2.md");
        assert_eq!(suggest_file(&d.path().join("note3.md")), None);
        // A directory with the right name is not a document.
        std::fs::create_dir(d.path().join("plans")).unwrap();
        assert_eq!(suggest_file(&d.path().join("plan")), None);
    }

    #[test]
    fn the_latest_document_is_the_one_written_last() {
        let at = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let entry = |p: &str, s: u64| crate::scan::Entry {
            path: PathBuf::from(p),
            mtime: at(s),
        };
        let entries = [entry("a.md", 5), entry("c.md", 9), entry("b.md", 9)];
        assert_eq!(latest(&entries), Some(Path::new("b.md")), "ties by path");
        assert_eq!(latest(&[]), None);
    }
}
