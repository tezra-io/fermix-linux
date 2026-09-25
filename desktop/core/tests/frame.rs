use fermix_client::frame::{read_frame, write_frame, FrameError, MAX_FRAME_BYTES};
use std::io::Cursor;

#[test]
fn a_frame_is_a_big_endian_length_then_the_bytes() {
    let mut out = Vec::new();
    write_frame(&mut out, b"{}").unwrap();
    assert_eq!(out, vec![0, 0, 0, 2, b'{', b'}']);
}

#[test]
fn a_written_frame_reads_back_whole() {
    let mut out = Vec::new();
    write_frame(&mut out, br#"{"method":"hello"}"#).unwrap();
    let body = read_frame(&mut Cursor::new(out)).unwrap();
    assert_eq!(body, br#"{"method":"hello"}"#);
}

#[test]
fn an_oversize_body_is_refused_before_anything_is_sent() {
    let mut out = Vec::new();
    let body = vec![b' '; MAX_FRAME_BYTES + 1];
    let err = write_frame(&mut out, &body).unwrap_err();
    assert!(matches!(err, FrameError::TooLarge(n) if n == MAX_FRAME_BYTES + 1));
    assert!(out.is_empty(), "nothing may be written for a refused frame");
}

#[test]
fn an_oversize_length_header_is_refused_without_allocating_it() {
    let header = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes();
    let err = read_frame(&mut Cursor::new(header.to_vec())).unwrap_err();
    assert!(matches!(err, FrameError::TooLarge(_)));
}

#[test]
fn a_frame_cut_short_is_an_io_error_not_a_short_body() {
    let mut bytes = vec![0, 0, 0, 10];
    bytes.extend_from_slice(b"abc");
    let err = read_frame(&mut Cursor::new(bytes)).unwrap_err();
    assert!(matches!(err, FrameError::Io(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof));
}
