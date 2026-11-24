//! Reading a stream of binary PPM pictures (`ffmpeg -f image2pipe -c:v ppm -`): every frame
//! carries its own size ("P6 <w> <h> 255" + RGB bytes), so the reader never has to guess it.

use std::io::{self, BufRead};

/// Largest frame accepted (pixels); anything bigger is a broken stream.
const MAX_PIXELS: usize = 4096 * 4096;

/// One RGB frame.
#[derive(Debug, PartialEq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

fn token(r: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut out = String::new();
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return Ok((!out.is_empty()).then_some(out));
        }
        let b = buf[0];
        if b == b'#' && out.is_empty() {
            // Comment up to the end of the line.
            let mut skip = Vec::new();
            r.read_until(b'\n', &mut skip)?;
            continue;
        }
        r.consume(1);
        if b.is_ascii_whitespace() {
            if out.is_empty() {
                continue;
            }
            return Ok(Some(out));
        }
        out.push(b as char);
        if out.len() > 16 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a PPM header"));
        }
    }
}

/// The next frame, `None` at a clean end of the stream.
pub fn read_frame(r: &mut impl BufRead) -> io::Result<Option<Frame>> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let Some(magic) = token(r)? else {
        return Ok(None);
    };
    if magic != "P6" {
        return Err(bad("not a binary PPM frame"));
    }
    let mut num =
        || -> io::Result<usize> { token(r)?.and_then(|t| t.parse().ok()).ok_or_else(|| bad("bad PPM header")) };
    let (w, h, max) = (num()?, num()?, num()?);
    // `token` consumed the single whitespace byte after the max value.
    if w == 0 || h == 0 || w * h > MAX_PIXELS || max == 0 || max > 255 {
        return Err(bad("unsupported PPM frame"));
    }
    let mut rgb = vec![0u8; w * h * 3];
    r.read_exact(&mut rgb)?;
    Ok(Some(Frame { w, h, rgb }))
}

/// A frame as PPM bytes (the fake producer and tests).
pub fn encode(f: &Frame) -> Vec<u8> {
    let mut v = format!("P6\n{} {}\n255\n", f.w, f.h).into_bytes();
    v.extend_from_slice(&f.rgb);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, v: u8) -> Frame {
        Frame { w, h, rgb: vec![v; w * h * 3] }
    }

    #[test]
    fn reads_a_stream_of_frames() {
        let mut bytes = encode(&frame(4, 2, 7));
        bytes.extend(encode(&frame(2, 2, 200)));
        bytes.extend(b"P6 # comment\n1 1 255\n\x01\x02\x03");
        let mut r = io::Cursor::new(bytes);
        assert_eq!(read_frame(&mut r).unwrap(), Some(frame(4, 2, 7)));
        assert_eq!(read_frame(&mut r).unwrap(), Some(frame(2, 2, 200)));
        assert_eq!(read_frame(&mut r).unwrap(), Some(Frame { w: 1, h: 1, rgb: vec![1, 2, 3] }));
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn broken_streams_are_errors() {
        assert!(read_frame(&mut io::Cursor::new(b"P5\n1 1\n255\n\x00".to_vec())).is_err());
        assert!(read_frame(&mut io::Cursor::new(b"P6\n0 1\n255\n".to_vec())).is_err());
        assert!(read_frame(&mut io::Cursor::new(b"P6\n99999 99999\n255\n".to_vec())).is_err());
        // Cut off in the middle of the pixels.
        assert!(read_frame(&mut io::Cursor::new(b"P6\n2 2\n255\n\x00\x00".to_vec())).is_err());
        assert!(read_frame(&mut io::Cursor::new(b"garbagegarbagegarbage".to_vec())).is_err());
    }
}
