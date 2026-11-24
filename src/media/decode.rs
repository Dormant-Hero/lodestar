//! Decoding sound files (wav / ogg / mp3 / flac, pure Rust) into the mixer's format: interleaved
//! stereo `f32` at `RATE`.

use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// The output rate (what the player process is opened with).
pub const RATE: u32 = 48_000;

/// Linear resampler + channel mapping to stereo, keeping its state across blocks.
pub struct Resampler {
    from: u32,
    /// Position of the next output frame, in input frames from `last`.
    pos: f64,
    last: [f32; 2],
    primed: bool,
}

impl Resampler {
    pub fn new(from: u32) -> Resampler {
        Resampler { from: from.max(1), pos: 0.0, last: [0.0; 2], primed: false }
    }

    /// Append `input` (`ch` channels interleaved) to `out` as stereo at `RATE`.
    pub fn push(&mut self, input: &[f32], ch: usize, out: &mut Vec<f32>) {
        let ch = ch.max(1);
        let frame = |i: usize| -> [f32; 2] {
            let f = &input[i * ch..i * ch + ch];
            if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] }
        };
        let n = input.len() / ch;
        if n == 0 {
            return;
        }
        if self.from == RATE {
            for i in 0..n {
                out.extend_from_slice(&frame(i));
            }
            self.last = frame(n - 1);
            return;
        }
        if !self.primed {
            self.last = frame(0);
            self.primed = true;
        }
        let step = self.from as f64 / RATE as f64;
        // Frames: -1 = `last`, 0.. = input.
        loop {
            let i = self.pos.floor();
            let t = (self.pos - i) as f32;
            let i = i as i64;
            if i + 1 >= n as i64 {
                break;
            }
            let a = if i < 0 { self.last } else { frame(i as usize) };
            let b = frame((i + 1) as usize);
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.pos += step;
        }
        // Next block: this block's last frame becomes frame -1.
        self.pos -= n as f64;
        self.last = frame(n - 1);
    }
}

/// An open file being decoded block by block.
pub struct Stream {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track: u32,
    resampler: Option<Resampler>,
    buf: Option<SampleBuffer<f32>>,
}

impl Stream {
    pub fn open(path: &Path) -> Option<Stream> {
        let file = File::open(path).ok()?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(e) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(e);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .ok()?;
        let format = probed.format;
        let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
        let decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
        let track = track.id;
        Some(Stream { format, decoder, track, resampler: None, buf: None })
    }

    /// Decode the next packet(s) into `out` (stereo, `RATE`). False at the end or on an error.
    pub fn next_block(&mut self, out: &mut Vec<f32>) -> bool {
        for _ in 0..64 {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(_) => return false,
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            };
            let spec = *decoded.spec();
            let ch = spec.channels.count();
            if ch == 0 || decoded.frames() == 0 {
                continue;
            }
            let buf = match &mut self.buf {
                Some(b) if b.capacity() >= decoded.capacity() * ch => b,
                _ => self.buf.insert(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)),
            };
            buf.copy_interleaved_ref(decoded);
            let rs = self.resampler.get_or_insert_with(|| Resampler::new(spec.rate));
            rs.push(buf.samples(), ch, out);
            return true;
        }
        false
    }
}

/// A whole (short) file, at most `max_secs` long. None when it can't be read.
pub fn decode_file(path: &Path, max_secs: f32) -> Option<Vec<f32>> {
    let mut s = Stream::open(path)?;
    let cap = (max_secs * RATE as f32) as usize * 2;
    let mut out = Vec::new();
    while out.len() < cap && s.next_block(&mut out) {}
    out.truncate(cap);
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
pub(crate) fn write_wav(path: &Path, rate: u32, ch: u16, samples: &[i16]) {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut v = Vec::new();
    v.extend(b"RIFF");
    v.extend((36 + data.len() as u32).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(ch.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * ch as u32 * 2).to_le_bytes());
    v.extend((ch * 2).to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend((data.len() as u32).to_le_bytes());
    v.extend(data);
    std::fs::write(path, v).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_wav_to_stereo_48k() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.wav");
        // 0.1 s mono 44.1 kHz at half scale.
        write_wav(&p, 44_100, 1, &vec![16384; 4410]);
        let pcm = decode_file(&p, 10.0).unwrap();
        let frames = pcm.len() / 2;
        assert!((4790..=4810).contains(&frames), "{frames} frames ≈ 0.1 s at 48 kHz");
        assert!(pcm.iter().all(|s| (s - 0.5).abs() < 0.01), "mono copied to both sides, value kept");
        // The cap.
        assert_eq!(decode_file(&p, 0.01).unwrap().len(), 480 * 2);
    }

    #[test]
    fn unreadable_files_give_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.wav");
        std::fs::write(&p, b"not a sound").unwrap();
        assert!(decode_file(&p, 1.0).is_none());
        assert!(decode_file(&d.path().join("missing.ogg"), 1.0).is_none());
    }

    #[test]
    fn resampler_is_continuous_across_blocks() {
        // A ramp split into blocks resamples to a smooth ramp.
        let ramp: Vec<f32> = (0..4410).map(|i| i as f32 / 4410.0).collect();
        let mut whole = vec![];
        Resampler::new(44_100).push(&ramp, 1, &mut whole);
        let mut parts = vec![];
        let mut r = Resampler::new(44_100);
        for c in ramp.chunks(333) {
            r.push(c, 1, &mut parts);
        }
        assert!((whole.len() as i64 - parts.len() as i64).abs() <= 4, "{} vs {}", whole.len(), parts.len());
        for w in parts.chunks(2).collect::<Vec<_>>().windows(2) {
            let d = w[1][0] - w[0][0];
            assert!((-1e-4..0.001).contains(&d), "monotonic, no jumps: {d}");
        }
    }
}
