//! Newline framing with a hard size limit.
//!
//! Every message is one UTF-8 JSON object followed by `\n`. A carriage return
//! before the newline is tolerated so clients on Windows that open a pipe in
//! text mode still work. Lines longer than [`MAX_LINE_BYTES`] are rejected
//! before they are buffered in full, which keeps a hostile local client from
//! exhausting server memory.

use std::fmt;
use std::io::{self, BufRead};

use serde::Serialize;

/// Longest accepted line, excluding the newline.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// A framing failure.
#[derive(Debug)]
pub enum FrameError {
    /// The underlying stream failed.
    Io(io::Error),
    /// The line exceeded [`MAX_LINE_BYTES`] before a newline was found.
    TooLong,
    /// The line was not valid UTF-8.
    InvalidUtf8,
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "read failed: {error}"),
            Self::TooLong => write!(
                formatter,
                "line exceeds the {MAX_LINE_BYTES} byte limit"
            ),
            Self::InvalidUtf8 => formatter.write_str("line is not valid UTF-8"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::TooLong | Self::InvalidUtf8 => None,
        }
    }
}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Reads newline-delimited UTF-8 lines with a size cap.
#[derive(Debug)]
pub struct LineReader<R> {
    inner: R,
    buffer: Vec<u8>,
}

impl<R: BufRead> LineReader<R> {
    /// Wrap a buffered reader.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(256),
        }
    }

    /// Read the next line without its terminator.
    ///
    /// Returns `Ok(None)` at end of stream. A final line without a trailing
    /// newline is returned normally, which makes hand-typed input work.
    pub fn read_line(&mut self) -> Result<Option<String>, FrameError> {
        self.buffer.clear();
        loop {
            let available = self.inner.fill_buf()?;
            if available.is_empty() {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return self.finish();
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.unwrap_or(available.len());
            if self.buffer.len() + take > MAX_LINE_BYTES {
                return Err(FrameError::TooLong);
            }
            self.buffer.extend_from_slice(&available[..take]);
            match newline {
                Some(index) => {
                    self.inner.consume(index + 1);
                    return self.finish();
                }
                None => {
                    self.inner.consume(take);
                }
            }
        }
    }

    fn finish(&mut self) -> Result<Option<String>, FrameError> {
        if self.buffer.last() == Some(&b'\r') {
            self.buffer.pop();
        }
        if self.buffer.len() > MAX_LINE_BYTES {
            return Err(FrameError::TooLong);
        }
        std::str::from_utf8(&self.buffer)
            .map(|line| Some(line.to_string()))
            .map_err(|_| FrameError::InvalidUtf8)
    }

    /// Give the inner reader back.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

/// Serialize a value as one line, including the trailing newline.
pub fn encode_line<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn reader(input: &[u8]) -> LineReader<BufReader<Cursor<Vec<u8>>>> {
        LineReader::new(BufReader::new(Cursor::new(input.to_vec())))
    }

    #[test]
    fn lines_are_split_on_newlines() {
        let mut reader = reader(b"one\ntwo\nthree\n");
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("one"));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("two"));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("three"));
        assert_eq!(reader.read_line().unwrap(), None);
    }

    #[test]
    fn a_trailing_line_without_a_newline_is_returned() {
        let mut reader = reader(b"one\ntwo");
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("one"));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("two"));
        assert_eq!(reader.read_line().unwrap(), None);
    }

    #[test]
    fn carriage_returns_are_stripped() {
        let mut reader = reader(b"one\r\ntwo\r\n");
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("one"));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn empty_lines_are_returned_as_empty_strings() {
        let mut reader = reader(b"\n\n");
        assert_eq!(reader.read_line().unwrap().as_deref(), Some(""));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some(""));
        assert_eq!(reader.read_line().unwrap(), None);
    }

    #[test]
    fn a_line_at_the_limit_is_accepted() {
        let mut input = vec![b'x'; MAX_LINE_BYTES];
        input.push(b'\n');
        let mut reader = reader(&input);
        let line = reader.read_line().unwrap().unwrap();
        assert_eq!(line.len(), MAX_LINE_BYTES);
    }

    #[test]
    fn a_line_over_the_limit_is_rejected() {
        let mut input = vec![b'x'; MAX_LINE_BYTES + 1];
        input.push(b'\n');
        let mut reader = reader(&input);
        assert!(matches!(
            reader.read_line(),
            Err(FrameError::TooLong)
        ));
    }

    #[test]
    fn an_unterminated_line_over_the_limit_is_rejected() {
        let input = vec![b'x'; MAX_LINE_BYTES + 1];
        let mut reader = reader(&input);
        assert!(matches!(
            reader.read_line(),
            Err(FrameError::TooLong)
        ));
    }

    #[test]
    fn invalid_utf8_is_reported() {
        let mut reader = reader(b"\xff\n");
        assert!(matches!(
            reader.read_line(),
            Err(FrameError::InvalidUtf8)
        ));
    }

    #[test]
    fn encoding_appends_exactly_one_newline() {
        let encoded = encode_line(&serde_json::json!({"a":1})).unwrap();
        assert_eq!(encoded.last(), Some(&b'\n'));
        assert_eq!(std::str::from_utf8(&encoded).unwrap().lines().count(), 1);
        assert!(std::str::from_utf8(&encoded).unwrap().starts_with('{'));
    }

    #[test]
    fn frame_errors_render_readable_messages() {
        assert!(!FrameError::TooLong.to_string().contains('\u{2014}'));
        assert!(FrameError::TooLong.to_string().contains("65536"));
        assert!(FrameError::InvalidUtf8.to_string().contains("UTF-8"));
    }
}
