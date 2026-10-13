//! The management socket's framing: a big-endian `u32` length, then exactly that
//! many bytes of UTF-8 JSON (Erlang `{:packet, 4}`). The daemon enforces the same ceiling.

use std::io::{self, Read, Write};

pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    TooLarge(usize),
    Io(io::Error),
}

pub fn write_frame(w: &mut impl Write, body: &[u8]) -> Result<(), FrameError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()));
    }
    let len = u32::try_from(body.len()).expect("MAX_FRAME_BYTES fits in a u32");
    w.write_all(&len.to_be_bytes()).map_err(FrameError::Io)?;
    w.write_all(body).map_err(FrameError::Io)?;
    w.flush().map_err(FrameError::Io)
}

pub fn read_frame(r: &mut impl Read) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; 4];
    r.read_exact(&mut header).map_err(FrameError::Io)?;
    let len = usize::try_from(u32::from_be_bytes(header)).expect("a u32 fits in usize");
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(FrameError::Io)?;
    Ok(body)
}
