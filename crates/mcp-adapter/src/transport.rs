//! The transport abstraction: how JSON-RPC messages reach an MCP server.
//!
//! A transport moves messages; it knows nothing about tools, manifests, or
//! the kernel. Sessions, handshakes, and MCP semantics live in
//! [`crate::client::McpClient`], which works over any implementation.

use std::io::BufRead;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::errors::McpError;

/// Largest single newline-delimited message any transport will buffer.
///
/// Both wire formats are line-oriented, and the standard readers grow their
/// buffer until a newline turns up. A server that never sends one — buggy or
/// hostile — would otherwise consume host memory without limit, which the
/// adapter's "a misbehaving server can never crash the host" guarantee does
/// not survive.
pub(crate) const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Read one `\n`-terminated line, refusing to buffer past [`MAX_MESSAGE_BYTES`].
///
/// Returns `Ok(None)` at end of stream. Unlike `BufRead::read_line`, the cap is
/// enforced *while* reading, so an unterminated flood is stopped rather than
/// accumulated and rejected afterwards. The trailing newline is not included.
pub(crate) fn read_line_capped<R: BufRead>(reader: &mut R) -> std::io::Result<Option<String>> {
    let mut line: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            // EOF: a final unterminated line is still a message.
            return if line.is_empty() {
                Ok(None)
            } else {
                decode_line(line).map(Some)
            };
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if line.len() + newline > MAX_MESSAGE_BYTES {
                return Err(oversized());
            }
            line.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            return decode_line(line).map(Some);
        }
        let chunk = available.len();
        if line.len() + chunk > MAX_MESSAGE_BYTES {
            return Err(oversized());
        }
        line.extend_from_slice(available);
        reader.consume(chunk);
    }
}

fn decode_line(line: Vec<u8>) -> std::io::Result<String> {
    String::from_utf8(line).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("MCP message is not valid UTF-8: {error}"),
        )
    })
}

fn oversized() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("MCP message exceeded {MAX_MESSAGE_BYTES} bytes without a newline"),
    )
}

/// Cooperative cancellation: the caller supplies a probe, the transport
/// checks it while waiting. `true` means stop now.
pub type CancelProbe = Arc<dyn Fn() -> bool + Send + Sync>;

/// Per-request budget and cancellation.
pub struct RequestOptions {
    pub timeout: Duration,
    pub cancel: Option<CancelProbe>,
}

impl RequestOptions {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            cancel: None,
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|probe| probe())
    }
}

pub trait McpTransport: Send + Sync {
    /// Send one JSON-RPC request and wait for its result. Every wait has a
    /// real deadline: a hung server produces `McpError::Timeout`, a fired
    /// cancel probe produces `McpError::Cancelled`.
    fn request(
        &self,
        method: &str,
        params: Value,
        options: &RequestOptions,
    ) -> Result<Value, McpError>;

    /// Send a fire-and-forget JSON-RPC notification.
    fn notify(&self, method: &str, params: Value) -> Result<(), McpError>;

    /// Terminate the connection cleanly (kill the child process, end the
    /// HTTP session). Idempotent; also invoked on drop by implementations.
    fn shutdown(&self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn reads_newline_delimited_messages() {
        let data = b"first\nsecond\n".to_vec();
        let mut reader = BufReader::new(&data[..]);
        assert_eq!(
            read_line_capped(&mut reader).unwrap().as_deref(),
            Some("first")
        );
        assert_eq!(
            read_line_capped(&mut reader).unwrap().as_deref(),
            Some("second")
        );
        assert_eq!(read_line_capped(&mut reader).unwrap(), None);
    }

    #[test]
    fn returns_a_final_unterminated_line() {
        let data = b"no trailing newline".to_vec();
        let mut reader = BufReader::new(&data[..]);
        assert_eq!(
            read_line_capped(&mut reader).unwrap().as_deref(),
            Some("no trailing newline")
        );
        assert_eq!(read_line_capped(&mut reader).unwrap(), None);
    }

    #[test]
    fn refuses_a_message_that_never_ends() {
        // A server that streams without ever sending a newline must be cut off
        // rather than buffered until the host runs out of memory.
        let data = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(&data[..]);
        let error = read_line_capped(&mut reader).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn accepts_a_message_exactly_at_the_cap() {
        let mut data = vec![b'x'; MAX_MESSAGE_BYTES];
        data.push(b'\n');
        let mut reader = BufReader::new(&data[..]);
        let line = read_line_capped(&mut reader).unwrap().unwrap();
        assert_eq!(line.len(), MAX_MESSAGE_BYTES);
    }

    #[test]
    fn refuses_invalid_utf8_before_a_newline() {
        let data = [b'{', 0xff, b'}', b'\n'];
        let mut reader = BufReader::new(&data[..]);

        let error = read_line_capped(&mut reader).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn refuses_invalid_utf8_at_end_of_stream() {
        let data = [b'{', 0xff, b'}'];
        let mut reader = BufReader::new(&data[..]);

        let error = read_line_capped(&mut reader).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
