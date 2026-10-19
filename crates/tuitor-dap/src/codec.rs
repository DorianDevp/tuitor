//! `Content-Length` framing used by DAP on both stdio and TCP transports.
//!
//! Each message is `Content-Length: N\r\n\r\n` followed by N bytes of JSON.
//! Other header fields are allowed by the spec and ignored here.

use bytes::{BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

const HEADER_END: &[u8] = b"\r\n\r\n";
/// Guard against a peer that never terminates its header block.
const MAX_HEADER_LEN: usize = 8 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("message header has no Content-Length")]
    MissingContentLength,
    #[error("invalid Content-Length `{0}`")]
    InvalidContentLength(String),
    #[error("message header exceeds {MAX_HEADER_LEN} bytes")]
    HeaderTooLong,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Splits a byte stream into raw JSON payloads and frames outgoing payloads.
#[derive(Debug, Default)]
pub struct DapCodec {
    /// Length of the body we are waiting for, once its header has been read.
    body_len: Option<usize>,
}

impl Decoder for DapCodec {
    type Item = Bytes;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, CodecError> {
        if self.body_len.is_none() {
            let Some(end) = find(src, HEADER_END) else {
                if src.len() > MAX_HEADER_LEN {
                    return Err(CodecError::HeaderTooLong);
                }
                return Ok(None);
            };
            let header = src.split_to(end + HEADER_END.len());
            self.body_len = Some(content_length(&header[..end])?);
        }

        let Some(len) = self.body_len else {
            return Ok(None);
        };
        if src.len() < len {
            src.reserve(len - src.len());
            return Ok(None);
        }
        self.body_len = None;
        Ok(Some(src.split_to(len).freeze()))
    }
}

impl Encoder<Bytes> for DapCodec {
    type Error = CodecError;

    fn encode(&mut self, body: Bytes, dst: &mut BytesMut) -> Result<(), CodecError> {
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        dst.reserve(header.len() + body.len());
        dst.put_slice(header.as_bytes());
        dst.put_slice(&body);
        Ok(())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn content_length(header: &[u8]) -> Result<usize, CodecError> {
    let header = String::from_utf8_lossy(header);
    for line in header.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            return value.parse().map_err(|_| CodecError::InvalidContentLength(value.to_owned()));
        }
    }
    Err(CodecError::MissingContentLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }

    fn decode_all(input: &[u8]) -> Vec<String> {
        let mut codec = DapCodec::default();
        let mut buf = BytesMut::from(input);
        let mut out = Vec::new();
        while let Some(msg) = codec.decode(&mut buf).expect("valid stream") {
            out.push(String::from_utf8(msg.to_vec()).expect("utf8"));
        }
        out
    }

    #[test]
    fn decodes_back_to_back_messages() {
        let input = frame(r#"{"seq":1}"#) + &frame(r#"{"seq":2}"#);
        assert_eq!(decode_all(input.as_bytes()), [r#"{"seq":1}"#, r#"{"seq":2}"#]);
    }

    #[test]
    fn waits_for_split_header_and_body() {
        let input = frame(r#"{"event":"stopped"}"#);
        let (a, b) = input.as_bytes().split_at(10);
        let mut codec = DapCodec::default();
        let mut buf = BytesMut::from(a);
        assert!(codec.decode(&mut buf).expect("partial header").is_none());
        buf.extend_from_slice(&b[..b.len() - 3]);
        assert!(codec.decode(&mut buf).expect("partial body").is_none());
        buf.extend_from_slice(&b[b.len() - 3..]);
        let msg = codec.decode(&mut buf).expect("complete").expect("message");
        assert_eq!(&msg[..], br#"{"event":"stopped"}"#);
    }

    #[test]
    fn ignores_other_headers_and_case() {
        let input = "content-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
        assert_eq!(decode_all(input.as_bytes()), ["{}"]);
    }

    #[test]
    fn counts_bytes_not_chars() {
        let body = r#"{"output":"zażółć"}"#;
        assert_eq!(decode_all(frame(body).as_bytes()), [body]);
    }

    #[test]
    fn rejects_header_without_length() {
        let mut codec = DapCodec::default();
        let mut buf = BytesMut::from(&b"X-Foo: 1\r\n\r\n{}"[..]);
        assert!(matches!(codec.decode(&mut buf), Err(CodecError::MissingContentLength)));
    }

    #[test]
    fn encodes_with_byte_length() {
        let mut codec = DapCodec::default();
        let mut dst = BytesMut::new();
        codec.encode(Bytes::from_static("ż".as_bytes()), &mut dst).expect("encode");
        assert_eq!(&dst[..], "Content-Length: 2\r\n\r\nż".as_bytes());
    }
}
