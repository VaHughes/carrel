//! stdin as a document: a reader thread and the UTF-8 seam it must respect.
//!
//! Chunk boundaries do not respect character boundaries, so a partial
//! multi-byte sequence at the end of a read is held back and prepended to
//! the next one — never lossily split. Genuinely invalid bytes become
//! U+FFFD; a reader shows what it got rather than dying on it.
//!
//! A pipe has no end it has to reach. Reading stops at [`LIMITS`] and says
//! so ([`Chunk::Stopped`]); see there for why.

use std::io::Read;
use std::sync::mpsc::{self, Receiver};

/// 64 KiB, matching the layout chunker's granularity.
const CHUNK: usize = 64 * 1024;

pub use carrel_state::text::Utf8Carry;

/// What the reader thread sends.
#[derive(Debug, PartialEq, Eq)]
pub enum Chunk {
    /// More of the document.
    Text(String),
    /// Reading stopped at a limit; nothing more will come. The pipe itself
    /// is left unread, not drained: draining an endless one never ends.
    Stopped(Stop),
}

/// Which limit a pipe reached.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// [`Limits::bytes`] of text.
    Bytes(usize),
    /// [`Limits::lines`] lines.
    Lines(usize),
}

impl Stop {
    /// The limit as a reader would say it: `64 MB`, `2,000,000 lines`.
    #[must_use]
    pub fn amount(self) -> String {
        match self {
            Self::Bytes(n) => format!("{} MB", n >> 20),
            Self::Lines(n) => {
                let digits = n.to_string();
                let mut out = String::new();
                for (i, c) in digits.chars().enumerate() {
                    if i > 0 && (digits.len() - i) % 3 == 0 {
                        out.push(',');
                    }
                    out.push(c);
                }
                format!("{out} lines")
            }
        }
    }
}

/// How much of a pipe the reader takes.
#[derive(Copy, Clone, Debug)]
pub struct Limits {
    pub bytes: usize,
    pub lines: usize,
}

/// The most of a pipe carrel reads.
///
/// A file is bounded by its size; a pipe is not, and every arriving batch
/// re-parses the whole document, so `yes '# x' | carrel` grew from 0.7 to
/// 4.2 GB in fifteen seconds and then aborted on a 300 MB allocation, the
/// terminal left in raw mode. Both bounds are needed: bytes for ordinary
/// text, lines for text made of tiny blocks — each `# x` line is a heading
/// costing some 230 bytes of model and layout for its 4 bytes of text, so
/// 64 MB of them would be four gigabytes. At 2,000,000 lines that case
/// stays near half a gigabyte, and a `git log -p` of ordinary width reaches
/// the byte limit first.
pub const LIMITS: Limits = Limits {
    bytes: 64 << 20,
    lines: 2_000_000,
};

/// Read `reader` to its end or to `limits`, handing each decoded piece to
/// `send`. A `false` from `send` means nobody is listening: stop at once.
///
/// The cut at a limit falls after the last whole line of the read that
/// fits, so a document is not ended mid-line where that can be helped, and
/// otherwise at the limit itself, on a character boundary.
pub fn pump(mut reader: impl Read, limits: Limits, mut send: impl FnMut(Chunk) -> bool) {
    let mut carry = Utf8Carry::default();
    let mut buf = vec![0u8; CHUNK];
    let mut bytes = 0usize;
    let mut lines = 0usize;
    loop {
        let s = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => carry.push(&buf[..n]),
        };
        if s.is_empty() {
            continue;
        }
        if let Some((cut, stop)) = cut_at(&s, bytes, lines, limits) {
            let head = &s[..cut];
            if !head.is_empty() && !send(Chunk::Text(head.to_owned())) {
                return;
            }
            send(Chunk::Stopped(stop));
            return;
        }
        bytes += s.len();
        lines += s.bytes().filter(|&b| b == b'\n').count();
        if !send(Chunk::Text(s)) {
            return; // receiver gone: the app quit
        }
    }
    let tail = carry.finish();
    if !tail.is_empty() {
        let _ = send(Chunk::Text(tail));
    }
}

/// Where `s` must be cut, if taking all of it would pass a limit, given
/// `bytes` and `lines` already taken.
fn cut_at(s: &str, bytes: usize, lines: usize, limits: Limits) -> Option<(usize, Stop)> {
    let room = limits.lines.saturating_sub(lines);
    // Just after the newline that ends the last line allowed. Exactly at
    // the limit is still under it: the stop comes with the next byte.
    let by_lines = if room == 0 {
        Some(0)
    } else {
        s.match_indices('\n')
            .nth(room - 1)
            .map(|(i, _)| i + 1)
            .filter(|&cut| cut < s.len())
    };
    let left = limits.bytes.saturating_sub(bytes);
    let by_bytes = (s.len() > left).then(|| {
        let mut at = left;
        while !s.is_char_boundary(at) {
            at -= 1;
        }
        // A line straddling the limit with no line end before it in this
        // read: up to the limit itself. The text already sent ends inside
        // that line, so stopping short of it would cut it no cleaner.
        s[..at].rfind('\n').map_or(at, |i| i + 1)
    });
    match (by_lines, by_bytes) {
        (Some(l), Some(b)) if l <= b => Some((l, Stop::Lines(limits.lines))),
        (_, Some(b)) => Some((b, Stop::Bytes(limits.bytes))),
        (Some(l), None) => Some((l, Stop::Lines(limits.lines))),
        (None, None) => None,
    }
}

/// Read `reader` to its end or to `limits`, as BYTES, for the modes that
/// print a pipe and exit.
///
/// They used to read all of stdin whatever its size, so the limit the
/// reader has was missing from `--plain -`, `--render -`, `- PATTERN` and
/// the bare pipe-out: `yes '# x' | carrel --plain -` read until memory ran
/// out. Bytes rather than text because those modes refuse input that looks
/// binary, which is a question about the bytes as they arrived.
///
/// The cut is [`pump`]'s: after the last whole line that fits, and
/// otherwise at the limit itself, never inside a character.
///
/// # Errors
///
/// Whatever reading `reader` returns.
pub fn read_capped(
    mut reader: impl Read,
    limits: Limits,
) -> std::io::Result<(Vec<u8>, Option<Stop>)> {
    let mut out: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut lines = 0usize;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok((out, None)),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let read = &buf[..n];
        // Just after the newline that ends the last line allowed, when
        // more follows it: exactly at the limit is still under it.
        let room = limits.lines.saturating_sub(lines);
        let by_lines = if room == 0 {
            Some(0)
        } else {
            read.iter()
                .enumerate()
                .filter(|&(_, &b)| b == b'\n')
                .nth(room - 1)
                .map(|(i, _)| i + 1)
                .filter(|&cut| cut < n)
        };
        let left = limits.bytes.saturating_sub(out.len());
        let by_bytes = (n > left).then(|| {
            let mut at = left;
            // A continuation byte right after the cut means the cut is
            // inside a character: back out of it.
            while at > 0 && read[at] & 0xC0 == 0x80 {
                at -= 1;
            }
            read[..at]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(at, |i| i + 1)
        });
        let (cut, stop) = match (by_lines, by_bytes) {
            (Some(l), Some(b)) if l <= b => (l, Stop::Lines(limits.lines)),
            (_, Some(b)) => (b, Stop::Bytes(limits.bytes)),
            (Some(l), None) => (l, Stop::Lines(limits.lines)),
            (None, None) => {
                lines += read.iter().map(|&b| usize::from(b == b'\n')).sum::<usize>();
                out.extend_from_slice(read);
                continue;
            }
        };
        out.extend_from_slice(&read[..cut]);
        return Ok((out, Some(stop)));
    }
}

/// Read stdin on a thread, sending decoded chunks. Dropping the sender is
/// the EOF signal, the same contract as `scan::spawn`'s channel.
#[must_use]
pub fn spawn() -> Receiver<Chunk> {
    // Backpressure keeps the decoded queue bounded while the UI parses or
    // the reader browses another document. Nothing is dropped.
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        pump(std::io::stdin().lock(), LIMITS, |chunk| {
            tx.send(chunk).is_ok()
        });
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that never ends, handing out `line` over and over in reads
    /// of an awkward size, so cuts land mid-line and mid-character.
    struct Endless {
        line: &'static [u8],
        at: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(1000);
            for b in &mut buf[..n] {
                *b = self.line[self.at % self.line.len()];
                self.at += 1;
            }
            Ok(n)
        }
    }

    fn pumped(reader: impl Read, limits: Limits) -> (String, Option<Stop>) {
        let mut text = String::new();
        let mut stop = None;
        pump(reader, limits, |chunk| {
            assert!(stop.is_none(), "nothing may follow Stopped");
            match chunk {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Stopped(s) => stop = Some(s),
            }
            true
        });
        (text, stop)
    }

    #[test]
    fn an_endless_pipe_of_headings_stops_at_the_line_limit() {
        let limits = Limits {
            bytes: 1 << 20,
            lines: 5_000,
        };
        let (text, stop) = pumped(
            Endless {
                line: b"# x\n",
                at: 0,
            },
            limits,
        );
        assert_eq!(stop, Some(Stop::Lines(5_000)));
        assert_eq!(
            text,
            "# x\n".repeat(5_000),
            "whole lines, exactly the limit"
        );
    }

    #[test]
    fn an_endless_pipe_of_words_stops_at_the_byte_limit_on_a_line_end() {
        let limits = Limits {
            bytes: 100_000,
            lines: usize::MAX,
        };
        let (text, stop) = pumped(
            Endless {
                line: "plain wörds\n".as_bytes(),
                at: 0,
            },
            limits,
        );
        assert_eq!(stop, Some(Stop::Bytes(100_000)));
        assert!(text.len() <= 100_000, "{}", text.len());
        assert!(
            text.len() > 100_000 - "plain wörds\n".len(),
            "at most a line short"
        );
        // A prefix of what was sent, whole lines but perhaps the last.
        let stream = "plain wörds\n".repeat(100_000 / 12 + 1);
        assert!(stream.starts_with(&text));
    }

    #[test]
    fn one_line_longer_than_the_limit_keeps_what_fits() {
        let limits = Limits {
            bytes: 10_001,
            lines: usize::MAX,
        };
        let (text, stop) = pumped(
            Endless {
                line: "é".as_bytes(),
                at: 0,
            },
            limits,
        );
        assert_eq!(stop, Some(Stop::Bytes(10_001)));
        assert_eq!(text, "é".repeat(5_000), "cut on a character boundary");
    }

    #[test]
    fn a_pipe_under_the_limits_arrives_whole_and_unremarked() {
        let src = "# a\n\nsome text\n".repeat(100);
        let (text, stop) = pumped(src.as_bytes(), LIMITS);
        assert_eq!(stop, None);
        assert_eq!(text, src);
        // Exactly at the line limit is still under it.
        let limits = Limits {
            bytes: usize::MAX,
            lines: 300,
        };
        let (text, stop) = pumped(src.as_bytes(), limits);
        assert_eq!((text.as_str(), stop), (src.as_str(), None));
    }

    /// The print-and-exit modes read bytes, and must stop where the
    /// streaming reader stops: same input, same limits, same text.
    #[test]
    fn the_byte_reader_stops_exactly_where_the_stream_does() {
        let cases: [(&[u8], Limits); 4] = [
            (
                b"# x\n",
                Limits {
                    bytes: 1 << 20,
                    lines: 5_000,
                },
            ),
            (
                "plain w\u{f6}rds\n".as_bytes(),
                Limits {
                    bytes: 100_000,
                    lines: usize::MAX,
                },
            ),
            (
                "\u{e9}".as_bytes(),
                Limits {
                    bytes: 10_001,
                    lines: usize::MAX,
                },
            ),
            (
                b"ab\n",
                Limits {
                    bytes: 3_000,
                    lines: 1_000,
                },
            ),
        ];
        for (line, limits) in cases {
            let (text, stop) = pumped(Endless { line, at: 0 }, limits);
            let (bytes, byte_stop) = read_capped(Endless { line, at: 0 }, limits).unwrap();
            assert!(stop.is_some(), "the fixture must reach a limit");
            assert_eq!(byte_stop, stop, "{limits:?}");
            assert_eq!(String::from_utf8(bytes).unwrap(), text, "{limits:?}");
        }
        // Under the limits: everything, and no stop.
        let src = "# a\n\nsome text\n".repeat(100);
        let limits = Limits {
            bytes: usize::MAX,
            lines: 300,
        };
        let (bytes, stop) = read_capped(src.as_bytes(), limits).unwrap();
        assert_eq!((bytes.as_slice(), stop), (src.as_bytes(), None));
    }

    #[test]
    fn the_limit_reads_as_a_person_would_say_it() {
        assert_eq!(Stop::Bytes(64 << 20).amount(), "64 MB");
        assert_eq!(Stop::Lines(2_000_000).amount(), "2,000,000 lines");
        assert_eq!(Stop::Lines(999).amount(), "999 lines");
    }

    #[test]
    fn a_full_chunk_of_invalid_bytes_is_decoded_without_recursion() {
        let mut carry = Utf8Carry::default();
        let decoded = carry.push(&vec![0xff; CHUNK]);
        assert_eq!(decoded, "\u{FFFD}".repeat(CHUNK));
        assert_eq!(carry.finish(), "");
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_bytes_match_lossy_utf8_independently_of_chunk_boundaries(
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048),
            chunk in 1usize..128,
        ) {
            let mut carry = Utf8Carry::default();
            let mut decoded = String::new();
            for part in bytes.chunks(chunk) {
                decoded.push_str(&carry.push(part));
            }
            decoded.push_str(&carry.finish());
            proptest::prop_assert_eq!(decoded, String::from_utf8_lossy(&bytes));
        }
    }

    #[test]
    fn a_multibyte_char_split_across_chunks_is_never_mangled() {
        let bytes = "héllo".as_bytes(); // é is 2 bytes
        let mut c = Utf8Carry::default();
        let mut out = String::new();
        out.push_str(&c.push(&bytes[..2])); // "h" + the first byte of é
        out.push_str(&c.push(&bytes[2..]));
        out.push_str(&c.finish());
        assert_eq!(out, "héllo");
    }

    #[test]
    fn a_four_byte_emoji_split_three_ways_survives() {
        let bytes = "a🦀b".as_bytes(); // 🦀 is 4 bytes
        let mut c = Utf8Carry::default();
        let mut out = String::new();
        for chunk in bytes.chunks(2) {
            out.push_str(&c.push(chunk));
        }
        out.push_str(&c.finish());
        assert_eq!(out, "a🦀b");
    }

    #[test]
    fn invalid_bytes_become_replacement_chars_not_a_panic() {
        let mut c = Utf8Carry::default();
        let out = format!("{}{}", c.push(b"ok\xFFgo"), c.finish());
        assert_eq!(out, "ok\u{FFFD}go");
    }

    #[test]
    fn a_dangling_carry_at_eof_is_flushed_lossily() {
        let mut c = Utf8Carry::default();
        let first = c.push(&"é".as_bytes()[..1]);
        assert_eq!(first, "", "nothing decodable yet");
        assert_eq!(c.finish(), "\u{FFFD}");
    }
}
