//! Bytes as text: the one rule every way into the reader shares.
//!
//! A file and a pipe used to disagree. `printf 'caf\xe9' | carrel` decoded
//! lossily and opened, while the same bytes through `--plain -`, `--render -`,
//! `carrel | cat` or as a FILE died with `stream did not contain valid
//! UTF-8` — so a Windows-1252 README with one stray byte could not be read
//! at all. Now every path decodes the same way: invalid sequences become
//! U+FFFD and the rest of the document reads, and only something that is
//! plainly not text (a NUL byte early on, or more replacement than text) is
//! refused.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 7.

/// How far into the bytes a NUL marks them as binary — git's own window
/// (`buffer_is_binary`), and the rule `grep` and `diff` follow too.
const SNIFF: usize = 8000;

/// Decodes a byte stream into `String`s across arbitrary chunk boundaries.
///
/// Chunk boundaries do not respect character boundaries, so a partial
/// multi-byte sequence at the end of a push is held back and prepended to
/// the next one — never lossily split. Genuinely invalid bytes become
/// U+FFFD, exactly as [`String::from_utf8_lossy`] would place them.
#[derive(Debug, Default)]
pub struct Utf8Carry {
    carry: Vec<u8>,
    replaced: usize,
}

impl Utf8Carry {
    /// Decode everything decodable, holding back a trailing partial char.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.carry.extend_from_slice(bytes);
        let buf = std::mem::take(&mut self.carry);
        let mut rest = buf.as_slice();
        let mut out = String::with_capacity(buf.len());
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    out.push_str(valid);
                    break;
                }
                Err(error) => {
                    let (valid, invalid) = rest.split_at(error.valid_up_to());
                    out.push_str(std::str::from_utf8(valid).expect("valid UTF-8 prefix"));
                    match error.error_len() {
                        // Only an incomplete trailing character crosses a read.
                        None => {
                            self.carry.extend_from_slice(invalid);
                            break;
                        }
                        Some(n) => {
                            out.push('\u{FFFD}');
                            self.replaced += 1;
                            rest = &invalid[n..];
                        }
                    }
                }
            }
        }
        out
    }

    /// How many invalid sequences have been replaced so far.
    #[must_use]
    pub const fn replaced(&self) -> usize {
        self.replaced
    }

    /// EOF: whatever is still held back will never complete; show it lossily.
    #[must_use]
    pub fn finish(self) -> String {
        String::from_utf8_lossy(&self.carry).into_owned()
    }
}

/// A whole document's bytes as text, and how many sequences were replaced.
///
/// Valid UTF-8 is taken as it is, without a copy. Anything else decodes
/// lossily — unless it is binary, which is an `InvalidData` error: a NUL in
/// the first 8000 bytes, or replacement characters making up more than a
/// quarter of what decoded. A Latin-1 page has one bad byte per accented
/// letter; an image or an archive is mostly bad bytes.
///
/// # Errors
///
/// `InvalidData` when the bytes are not UTF-8 and look binary.
pub fn decode_document(bytes: Vec<u8>) -> std::io::Result<(String, usize)> {
    let bytes = match String::from_utf8(bytes) {
        Ok(text) => return Ok((text, 0)),
        Err(e) => e.into_bytes(),
    };
    let not_text = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "this is not text — carrel reads markdown and other text files",
        )
    };
    if bytes[..bytes.len().min(SNIFF)].contains(&0) {
        return Err(not_text());
    }
    let mut carry = Utf8Carry::default();
    let mut text = carry.push(&bytes);
    let mut replaced = carry.replaced();
    let tail = carry.finish();
    if !tail.is_empty() {
        replaced += 1;
        text.push_str(&tail);
    }
    if replaced.saturating_mul(4) > text.chars().count() {
        return Err(not_text());
    }
    Ok((text, replaced))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_taken_whole_and_nothing_is_replaced() {
        let (text, n) = decode_document("café # ok\n".as_bytes().to_vec()).unwrap();
        assert_eq!((text.as_str(), n), ("café # ok\n", 0));
    }

    /// The Windows-1252 README with one stray byte: it reads, with the byte
    /// shown as U+FFFD and counted so the reader can say so.
    #[test]
    fn a_mostly_text_document_with_stray_bytes_decodes_lossily() {
        let (text, n) = decode_document(b"# caf\xe9\n\nna\xefve latin1\n".to_vec()).unwrap();
        assert_eq!(text, "# caf\u{FFFD}\n\nna\u{FFFD}ve latin1\n");
        assert_eq!(n, 2);
    }

    #[test]
    fn a_truncated_last_character_is_counted() {
        let (text, n) = decode_document(b"ok \xe2\x82".to_vec()).unwrap();
        assert_eq!((text.as_str(), n), ("ok \u{FFFD}", 1));
    }

    #[test]
    fn what_looks_binary_is_still_refused() {
        // A PNG's signature: invalid UTF-8 with a NUL soon after.
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        let e = decode_document(png).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        assert!(e.to_string().contains("not text"), "{e}");
        // No NUL, but more garbage than text.
        assert!(decode_document(vec![0xff; 64]).is_err());
        assert!(decode_document(b"ab\xff\xfe\xfd".to_vec()).is_err());
    }
}
