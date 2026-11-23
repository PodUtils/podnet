//! Carriers inside the bytes of an application (T-371, #494).
//!
//! A response of the deferred cache can carry a control stream without
//! changing the bytes of its body: each frame is a length-prefixed payload,
//! hex encoded, in the `X-Pod-Carrier` header of the HTTP/1.1 response. An
//! unknown header is legal in HTTP/1.1, so the application parser accepts
//! the response and the body is untouched.
//!
//! A frame is `[u32 big-endian length][payload]`, hex encoded; one header
//! line can hold several frames one after the other. The line is added only
//! when the response with it fits the chunk of the cache, so the bound of
//! DEFER-2 holds for the bytes that go on the wire as well.

use std::collections::VecDeque;

/// The header of a carrier frame.
pub const HEADER: &str = "x-pod-carrier";

/// The most bytes of one frame payload.
pub const MAX_FRAME: usize = 1024;

/// The carrier stream of one prepared response.
#[derive(Debug, Default)]
pub struct Carrier {
    frames: VecDeque<Vec<u8>>,
    sent: u64,
}

impl Carrier {
    /// An empty carrier.
    pub fn new() -> Carrier {
        Carrier::default()
    }

    /// Queue `payload` as the next frame. A payload over [`MAX_FRAME`] is
    /// split; the empty payload is ignored. Gives the frames queued.
    pub fn push(&mut self, payload: &[u8]) -> usize {
        let mut rest = payload;
        while !rest.is_empty() {
            let n = rest.len().min(MAX_FRAME);
            self.frames.push_back(rest[..n].to_vec());
            rest = &rest[n..];
        }
        self.frames.len()
    }

    /// The frames that wait.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when no frame waits.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The frames that a response carried.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// The next frame as its hex text, or `None`, without taking it.
    fn peek_hex(&self) -> Option<String> {
        let frame = self.frames.front()?;
        let mut bytes = Vec::with_capacity(4 + frame.len());
        bytes.extend_from_slice(&(frame.len() as u32).to_be_bytes());
        bytes.extend_from_slice(frame);
        Some(hex(&bytes))
    }

    /// Take the next frame.
    fn take(&mut self) {
        self.frames.pop_front();
        self.sent += 1;
    }

    /// `response` with the next frame in its head, when the result fits
    /// `chunk`. A response with no head, or no room after the frame, is
    /// returned unchanged and the frame waits.
    pub fn attach(&mut self, response: Vec<u8>, chunk: usize) -> Vec<u8> {
        let Some(at) = response.windows(4).position(|w| w == b"\r\n\r\n") else {
            return response;
        };
        let Some(text) = self.peek_hex() else {
            return response;
        };
        let line = format!("\r\n{HEADER}: {text}");
        if response.len() + line.len() > chunk {
            return response;
        }
        self.take();
        let mut out = Vec::with_capacity(response.len() + line.len());
        out.extend_from_slice(&response[..at]);
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(&response[at..]);
        out
    }
}

/// The frames of `response`, in order. The error names a header that is not
/// hex or a frame whose length is wrong.
pub fn frames(response: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::new();
    for line in head_lines(response) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case(HEADER) {
            continue;
        }
        let bytes = unhex(value.trim())?;
        let mut at = 0;
        while at < bytes.len() {
            if bytes.len() - at < 4 {
                return Err("a carrier frame has no length".to_string());
            }
            let len =
                u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four bytes")) as usize;
            at += 4;
            if bytes.len() - at < len {
                return Err("a carrier frame is shorter than its length".to_string());
            }
            out.push(bytes[at..at + len].to_vec());
            at += len;
        }
    }
    Ok(out)
}

/// The lines of the head of an HTTP response, after the status line.
fn head_lines(response: &[u8]) -> impl Iterator<Item = &str> {
    let end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(response.len());
    std::str::from_utf8(&response[..end])
        .unwrap_or("")
        .lines()
        .skip(1)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0xf)] as char);
    }
    out
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(2) {
        return Err("a carrier header has an odd number of hex digits".to_string());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in text.chunks(2) {
        let high = digit(pair[0])?;
        let low = digit(pair[1])?;
        out.push(high << 4 | low);
    }
    Ok(out)
}

fn digit(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("a carrier header is not hex".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";

    #[test]
    fn a_frame_round_trips_through_the_header() {
        let mut carrier = Carrier::new();
        carrier.push(b"hello");
        let got = carrier.attach(RESPONSE.to_vec(), 4096);
        assert!(got.ends_with(b"hi"));
        assert_eq!(frames(&got).unwrap(), vec![b"hello".to_vec()]);
        assert_eq!(carrier.sent(), 1);
    }

    #[test]
    fn no_room_in_the_chunk_leaves_the_frame_waiting() {
        let mut carrier = Carrier::new();
        carrier.push(b"hello");
        let got = carrier.attach(RESPONSE.to_vec(), RESPONSE.len());
        assert_eq!(got, RESPONSE);
        assert_eq!(carrier.len(), 1);
    }

    #[test]
    fn a_long_payload_is_split() {
        let mut carrier = Carrier::new();
        carrier.push(&vec![7u8; MAX_FRAME + 1]);
        assert_eq!(carrier.len(), 2);
    }

    #[test]
    fn several_frames_fill_one_header() {
        let mut carrier = Carrier::new();
        carrier.push(b"one");
        carrier.push(b"two");
        let got = carrier.attach(RESPONSE.to_vec(), 4096);
        // One frame per response, in order.
        let first = frames(&got).unwrap();
        assert_eq!(first, vec![b"one".to_vec()]);
        let got = carrier.attach(got, 4096);
        assert_eq!(
            frames(&got).unwrap(),
            vec![b"one".to_vec(), b"two".to_vec()]
        );
    }
}
