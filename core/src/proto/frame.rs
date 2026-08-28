//! Length-prefixed frame codec: 4-byte little-endian length, then UTF-8 JSON.

use std::io::{Read, Write};

/// Hard ceiling on one frame's payload, from the process contract.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Every way a frame can fail to cross the pipe.
#[derive(Debug)]
pub enum FrameError {
    /// The peer closed the pipe between frames. Orderly, not a protocol violation.
    Eof,
    /// The declared length exceeds `MAX_FRAME_BYTES`. The body is never read and never
    /// allocated; the connection is killed.
    Oversize {
        declared: u32,
    },
    /// The payload was not valid UTF-8.
    NotUtf8,
    /// The payload the caller asked to send exceeds `MAX_FRAME_BYTES`.
    PayloadTooLarge {
        len: usize,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eof => write!(f, "pipe closed between frames"),
            Self::Oversize { declared } => {
                write!(
                    f,
                    "frame length {declared} exceeds the {MAX_FRAME_BYTES}-byte cap"
                )
            }
            Self::NotUtf8 => write!(f, "frame payload was not valid UTF-8"),
            Self::PayloadTooLarge { len } => {
                write!(
                    f,
                    "payload of {len} bytes exceeds the {MAX_FRAME_BYTES}-byte cap"
                )
            }
            Self::Io(e) => write!(f, "pipe io: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Reads one frame into `buf`, replacing its contents. `buf` is reused across calls so a
/// steady stream of frames does not allocate per frame.
pub fn read_frame<R: Read>(reader: &mut R, buf: &mut Vec<u8>) -> Result<(), FrameError> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(FrameError::Eof),
        Err(e) => return Err(FrameError::Io(e)),
    }
    let declared = u32::from_le_bytes(header);
    let len = usize::try_from(declared).map_err(|_| FrameError::Oversize { declared })?;
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize { declared });
    }
    buf.clear();
    buf.try_reserve(len)
        .map_err(|_| FrameError::Oversize { declared })?;
    buf.resize(len, 0);
    reader.read_exact(buf).map_err(FrameError::Io)?;
    if std::str::from_utf8(buf).is_err() {
        return Err(FrameError::NotUtf8);
    }
    Ok(())
}

/// Writes one frame. The caller's payload must already be UTF-8 JSON.
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    let len = payload.len();
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::PayloadTooLarge { len });
    }
    let header = u32::try_from(len).map_err(|_| FrameError::PayloadTooLarge { len })?;
    writer.write_all(&header.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{read_frame, write_frame, FrameError, MAX_FRAME_BYTES};
    use std::io::Read;

    /// Yields the four header bytes, then errors on any further read. If the decoder touches
    /// the body after an oversize header, this reader says so instead of allocating 4 GiB.
    struct HeaderOnly {
        header: [u8; 4],
        pos: usize,
    }

    impl Read for HeaderOnly {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= 4 {
                return Err(std::io::Error::other("body must not be read"));
            }
            let Some(slot) = out.first_mut() else {
                return Ok(0);
            };
            let Some(byte) = self.header.get(self.pos) else {
                return Ok(0);
            };
            *slot = *byte;
            self.pos += 1;
            Ok(1)
        }
    }

    #[test]
    fn round_trips_a_frame() {
        let mut wire = Vec::new();
        write_frame(&mut wire, b"{\"t\":\"hello\"}").expect("write");
        let mut buf = Vec::new();
        read_frame(&mut wire.as_slice(), &mut buf).expect("read");
        assert_eq!(buf, b"{\"t\":\"hello\"}");
    }

    #[test]
    fn a_malformed_length_is_rejected_without_reading_the_body() {
        let mut reader = HeaderOnly {
            header: u32::MAX.to_le_bytes(),
            pos: 0,
        };
        let mut buf = Vec::new();
        match read_frame(&mut reader, &mut buf) {
            Err(FrameError::Oversize { declared }) => assert_eq!(declared, u32::MAX),
            other => panic!("expected Oversize, got {other:?}"),
        }
        assert!(buf.is_empty(), "no allocation for an oversize frame");
    }

    #[test]
    fn a_payload_over_the_cap_is_refused_at_the_writer() {
        let mut sink = Vec::new();
        let big = vec![b'x'; MAX_FRAME_BYTES + 1];
        match write_frame(&mut sink, &big) {
            Err(FrameError::PayloadTooLarge { len }) => assert_eq!(len, MAX_FRAME_BYTES + 1),
            other => panic!("expected PayloadTooLarge, got {other:?}"),
        }
        assert!(sink.is_empty(), "nothing reaches the pipe");
    }

    #[test]
    fn a_closed_pipe_between_frames_is_eof_not_an_error() {
        let mut buf = Vec::new();
        match read_frame(&mut std::io::empty(), &mut buf) {
            Err(FrameError::Eof) => {}
            other => panic!("expected Eof, got {other:?}"),
        }
    }
}
