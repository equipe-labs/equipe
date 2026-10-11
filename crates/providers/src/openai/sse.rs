//! Incremental Server-Sent Events decoder.
//!
//! Frames are split on raw bytes, so a multi-byte UTF-8 character that straddles
//! two network chunks is reassembled before it is decoded.

use serde_json::Value;

use crate::error::{Error, Result};

#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; returns the events whose frames are now complete.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((at, len)) = find_boundary(&self.buf) {
            let frame: Vec<u8> = self.buf.drain(..at + len).take(at).collect();
            decode_frame(&frame, &mut events)?;
        }
        Ok(events)
    }

    /// End of stream: whatever is left is the last frame, delimiter or not.
    pub fn finish(&mut self) -> Result<Vec<Value>> {
        let frame = std::mem::take(&mut self.buf);
        let mut events = Vec::new();
        decode_frame(&frame, &mut events)?;
        Ok(events)
    }
}

/// Earliest blank line (`\n\n`, `\r\n\r\n` or `\r\r`): its index and length.
fn find_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    (0..buf.len()).find_map(|i| {
        let rest = &buf[i..];
        if rest.starts_with(b"\r\n\r\n") {
            Some((i, 4))
        } else if rest.starts_with(b"\n\n") || rest.starts_with(b"\r\r") {
            Some((i, 2))
        } else {
            None
        }
    })
}

fn decode_frame(frame: &[u8], out: &mut Vec<Value>) -> Result<()> {
    let text = String::from_utf8_lossy(frame);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let data = normalized
        .split('\n')
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    let event = serde_json::from_str(data).map_err(|_| {
        let preview: String = data.chars().take(120).collect();
        Error::Protocol(format!("malformed SSE frame: {preview}"))
    })?;
    out.push(event);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode(chunks: &[&[u8]]) -> Result<Vec<Value>> {
        let mut decoder = SseDecoder::new();
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(decoder.push(chunk)?);
        }
        events.extend(decoder.finish()?);
        Ok(events)
    }

    #[test]
    fn decodes_frames_and_drops_the_done_sentinel() {
        let events = decode(&[
            b"data: {\"type\":\"a\"}\n\n",
            b"data: {\"type\":\"b\"}\n\n",
            b"data: [DONE]\n\n",
        ])
        .unwrap();
        assert_eq!(events, vec![json!({"type": "a"}), json!({"type": "b"})]);
    }

    #[test]
    fn reassembles_frames_split_across_chunks() {
        // Network chunks land wherever they land — mid-JSON, mid-delimiter.
        let events = decode(&[
            b"data: {\"ty",
            b"pe\":\"a\",\"delta\":\"hi\"}",
            b"\n",
            b"\ndata: {\"type\":\"b\"}\n\n",
        ])
        .unwrap();
        assert_eq!(
            events,
            vec![json!({"type": "a", "delta": "hi"}), json!({"type": "b"})]
        );
    }

    #[test]
    fn reassembles_a_utf8_character_split_across_chunks() {
        let frame = "data: {\"delta\":\"안녕\"}\n\n".as_bytes();
        // Split inside the three-byte encoding of '안'.
        let split = frame.iter().position(|&b| b >= 0x80).unwrap() + 1;
        let events = decode(&[&frame[..split], &frame[split..]]).unwrap();
        assert_eq!(events, vec![json!({"delta": "안녕"})]);
    }

    #[test]
    fn handles_crlf_delimiters() {
        let events = decode(&[b"data: {\"type\":\"a\"}\r\n\r\ndata: {\"type\":\"b\"}\r\n\r\n"]).unwrap();
        assert_eq!(events, vec![json!({"type": "a"}), json!({"type": "b"})]);
    }

    #[test]
    fn emits_a_final_frame_without_a_trailing_blank_line() {
        let events = decode(&[b"data: {\"type\":\"a\"}"]).unwrap();
        assert_eq!(events, vec![json!({"type": "a"})]);
    }

    #[test]
    fn ignores_comments_event_names_and_blank_frames() {
        let events = decode(&[
            b": keep-alive\n\n",
            b"event: ping\ndata: {\"type\":\"a\"}\n\n",
            b"\n\n",
        ])
        .unwrap();
        assert_eq!(events, vec![json!({"type": "a"})]);
    }

    #[test]
    fn rejects_a_malformed_frame() {
        let err = decode(&[b"data: {not json}\n\n"]).unwrap_err();
        assert!(err.to_string().contains("malformed SSE frame"));
    }
}
