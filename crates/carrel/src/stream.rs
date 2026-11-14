//! stdin as a document: a reader thread and the UTF-8 seam it must respect.
//!
//! Chunk boundaries do not respect character boundaries, so a partial
//! multi-byte sequence at the end of a read is held back and prepended to
//! the next one — never lossily split. Genuinely invalid bytes become
//! U+FFFD; a reader shows what it got rather than dying on it.

use std::io::Read;
use std::sync::mpsc::{self, Receiver};

/// 64 KiB, matching the layout chunker's granularity.
const CHUNK: usize = 64 * 1024;

pub use carrel_state::text::Utf8Carry;

/// Read stdin on a thread, sending decoded chunks. Dropping the sender is
/// the EOF signal, the same contract as `scan::spawn`'s channel.
#[must_use]
pub fn spawn() -> Receiver<String> {
    // Backpressure keeps the decoded queue bounded while the UI parses or
    // the reader browses another document. Nothing is dropped.
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut carry = Utf8Carry::default();
        let mut buf = vec![0u8; CHUNK];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let s = carry.push(&buf[..n]);
                    if !s.is_empty() && tx.send(s).is_err() {
                        return; // receiver gone: the app quit
                    }
                }
            }
        }
        let tail = carry.finish();
        if !tail.is_empty() {
            let _ = tx.send(tail);
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

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
