//! Packet-4 framing.
//!
//! One big-endian 32-bit byte length followed by exactly that many bytes of
//! UTF-8 JSON. The length is validated against the contract's ceiling *before*
//! anything is allocated, so a hostile or broken peer cannot make this process
//! ask the allocator for four gigabytes.

use gtk4::gio;
use gtk4::prelude::*;

use super::contract;

/// What a frame read or write can refuse.
#[derive(Debug)]
pub enum FrameError {
    /// The peer closed before the frame was whole.
    Truncated { expected: usize, read: usize },
    /// The declared length is above the contract's ceiling.
    TooLarge { declared: usize, ceiling: usize },
    /// The stream itself failed.
    Io(glib::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Truncated { expected, read } => {
                write!(
                    formatter,
                    "frame truncated: expected {expected} bytes, read {read}"
                )
            }
            FrameError::TooLarge { declared, ceiling } => {
                write!(
                    formatter,
                    "frame declares {declared} bytes, ceiling is {ceiling}"
                )
            }
            FrameError::Io(error) => write!(formatter, "stream failed: {error}"),
        }
    }
}

impl std::error::Error for FrameError {}

use gtk4::glib;

/// The frame ceiling, from the vendored contract.
pub fn ceiling() -> usize {
    contract::limits().max_frame_bytes
}

/// Encode one frame. Refuses a body above the ceiling before it is sent, so an
/// oversized request never becomes an opaque failure on the far side.
pub fn encode(body: &[u8]) -> Result<Vec<u8>, FrameError> {
    let ceiling = ceiling();
    if body.len() > ceiling {
        return Err(FrameError::TooLarge {
            declared: body.len(),
            ceiling,
        });
    }

    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Read one frame from a stream.
pub async fn read_frame(stream: &gio::InputStream) -> Result<Vec<u8>, FrameError> {
    let header = read_exactly(stream, 4).await?;
    let declared = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;

    let ceiling = ceiling();
    if declared > ceiling {
        return Err(FrameError::TooLarge { declared, ceiling });
    }

    read_exactly(stream, declared).await
}

/// Write one frame to a stream.
pub async fn write_frame(stream: &gio::OutputStream, body: &[u8]) -> Result<(), FrameError> {
    let frame = encode(body)?;
    stream
        .write_all_future(frame, glib::Priority::DEFAULT)
        .await
        .map(|_| ())
        .map_err(|(_, error)| FrameError::Io(error))
}

/// Read exactly `wanted` bytes, or say how few arrived.
///
/// The loop is bounded by `wanted`, which is bounded by the ceiling checked
/// before the first read, and every iteration either consumes bytes or ends the
/// loop.
async fn read_exactly(stream: &gio::InputStream, wanted: usize) -> Result<Vec<u8>, FrameError> {
    let mut collected: Vec<u8> = Vec::with_capacity(wanted.min(64 * 1024));

    while collected.len() < wanted {
        let remaining = wanted - collected.len();
        let buffer = vec![0u8; remaining.min(64 * 1024)];
        let (buffer, read) = stream
            .read_future(buffer, glib::Priority::DEFAULT)
            .await
            .map_err(|(_, error)| FrameError::Io(error))?;

        if read == 0 {
            return Err(FrameError::Truncated {
                expected: wanted,
                read: collected.len(),
            });
        }
        collected.extend_from_slice(&buffer[..read]);
    }

    Ok(collected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::glib::MainContext;

    fn memory_stream(bytes: Vec<u8>) -> gio::InputStream {
        gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes)).upcast()
    }

    #[test]
    fn a_frame_round_trips() {
        let body = br#"{"request_id":"fx-1"}"#.to_vec();
        let frame = encode(&body).expect("encodes");

        let read = MainContext::new()
            .block_on(async { read_frame(&memory_stream(frame)).await })
            .expect("reads");

        assert_eq!(read, body);
    }

    #[test]
    fn an_oversized_body_is_refused_before_it_is_sent() {
        let body = vec![0u8; ceiling() + 1];
        match encode(&body) {
            Err(FrameError::TooLarge {
                declared,
                ceiling: limit,
            }) => {
                assert_eq!(declared, body.len());
                assert_eq!(limit, ceiling());
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_oversized_declared_length_is_refused_before_allocating() {
        // Four header bytes claiming a gigabyte, and nothing behind them. The
        // read must refuse on the header alone.
        let mut frame = (1_073_741_824u32).to_be_bytes().to_vec();
        frame.push(b'{');

        let result = MainContext::new().block_on(async { read_frame(&memory_stream(frame)).await });

        match result {
            Err(FrameError::TooLarge { declared, .. }) => assert_eq!(declared, 1_073_741_824),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_short_frame_is_truncated_not_silently_short() {
        let mut frame = (32u32).to_be_bytes().to_vec();
        frame.extend_from_slice(b"only eight");

        let result = MainContext::new().block_on(async { read_frame(&memory_stream(frame)).await });

        match result {
            Err(FrameError::Truncated { expected, read }) => {
                assert_eq!(expected, 32);
                assert_eq!(read, 10);
            }
            other => panic!("expected a truncation, got {other:?}"),
        }
    }

    #[test]
    fn a_header_that_never_arrives_is_truncated() {
        let result =
            MainContext::new().block_on(async { read_frame(&memory_stream(vec![0u8, 0u8])).await });

        match result {
            Err(FrameError::Truncated { expected, read }) => {
                assert_eq!(expected, 4);
                assert_eq!(read, 2);
            }
            other => panic!("expected a truncation, got {other:?}"),
        }
    }

    #[test]
    fn writing_a_frame_puts_the_length_in_front_of_the_body() {
        let stream = gio::MemoryOutputStream::new_resizable();
        MainContext::new()
            .block_on(async { write_frame(&stream.clone().upcast(), b"hi").await })
            .expect("writes");
        stream.close(gio::Cancellable::NONE).expect("closes");

        assert_eq!(
            stream.steal_as_bytes().to_vec(),
            vec![0, 0, 0, 2, b'h', b'i']
        );
    }
}
