//! Mixing navigation sounds and background music into the player's sample stream.

use super::audio::Sound;
use super::decode::RATE;
use crossbeam_channel::{Receiver, TryRecvError};
use std::collections::HashMap;
use std::sync::Arc;

/// Sounds playing at once (more: the oldest stops).
const MAX_VOICES: usize = 8;
/// Music fades in / out over this long.
pub const FADE_SECS: f32 = 0.8;
/// Music sits under the navigation sounds.
const MUSIC_LEVEL: f32 = 0.55;

struct Voice {
    sound: Sound,
    pcm: Arc<Vec<f32>>,
    pos: usize,
}

/// Background music being played: blocks of samples from a decoder thread.
pub struct Music {
    rx: Receiver<Vec<f32>>,
    cur: Vec<f32>,
    pos: usize,
    /// The decoder finished (or failed) and everything it sent was played.
    pub ended: bool,
    /// Samples played (0 = the file gave nothing).
    pub played: usize,
}

impl Music {
    pub fn new(rx: Receiver<Vec<f32>>) -> Music {
        Music { rx, cur: Vec::new(), pos: 0, ended: false, played: 0 }
    }

    /// The next stereo sample pair, None when none is ready (still decoding: silence) or ended.
    fn next(&mut self) -> Option<[f32; 2]> {
        while self.pos + 1 >= self.cur.len() {
            match self.rx.try_recv() {
                Ok(b) => {
                    self.cur = b;
                    self.pos = 0;
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    return None;
                }
            }
        }
        let s = [self.cur[self.pos], self.cur[self.pos + 1]];
        self.pos += 2;
        self.played += 2;
        Some(s)
    }
}

#[derive(Default)]
pub struct Mixer {
    sounds: HashMap<Sound, Arc<Vec<f32>>>,
    voices: Vec<Voice>,
    /// 0..1 (already shaped by `gain_for`).
    pub volume: f32,
    pub music: Option<Music>,
    music_gain: f32,
    /// 1 = music on, 0 = fading out / off.
    pub music_target: f32,
}

/// Volume percent → gain (a squared curve sounds even across the range).
pub fn gain_for(percent: u8) -> f32 {
    let v = percent.min(100) as f32 / 100.0;
    v * v
}

impl Mixer {
    pub fn set_sounds(&mut self, s: HashMap<Sound, Arc<Vec<f32>>>) {
        self.sounds = s;
        self.voices.clear();
    }

    pub fn play(&mut self, s: Sound) {
        let Some(pcm) = self.sounds.get(&s).cloned() else {
            return;
        };
        // The same sound again (scrolling fast): restart it instead of piling up.
        self.voices.retain(|v| v.sound != s);
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(Voice { sound: s, pcm, pos: 0 });
    }

    pub fn stop_sounds(&mut self) {
        self.voices.clear();
    }

    /// Something can be heard (or is about to be): sounds playing or music not faded out.
    pub fn busy(&self) -> bool {
        !self.voices.is_empty() || self.music.is_some() && (self.music_gain > 0.0 || self.music_target > 0.0)
    }

    /// The music has faded out completely.
    pub fn music_silent(&self) -> bool {
        self.music_gain <= 0.0 && self.music_target <= 0.0
    }

    /// Fill `out` (interleaved stereo) with the next samples.
    pub fn mix(&mut self, out: &mut [i16]) {
        let step = 1.0 / (FADE_SECS * RATE as f32);
        for frame in out.as_chunks_mut::<2>().0 {
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for v in &mut self.voices {
                if v.pos + 1 < v.pcm.len() {
                    l += v.pcm[v.pos];
                    r += v.pcm[v.pos + 1];
                    v.pos += 2;
                }
            }
            if self.music_gain != self.music_target {
                self.music_gain = if self.music_gain < self.music_target {
                    (self.music_gain + step).min(self.music_target)
                } else {
                    (self.music_gain - step).max(self.music_target)
                };
            }
            if self.music_gain > 0.0
                && let Some(m) = &mut self.music
                && let Some([ml, mr]) = m.next()
            {
                let g = self.music_gain * self.music_gain * MUSIC_LEVEL;
                l += ml * g;
                r += mr * g;
            }
            frame[0] = to_i16(l * self.volume);
            frame[1] = to_i16(r * self.volume);
        }
        self.voices.retain(|v| v.pos + 1 < v.pcm.len());
    }
}

fn to_i16(v: f32) -> i16 {
    // Soft limit instead of hard clipping when several sounds add up.
    let v = if v.abs() > 0.9 { v.signum() * (0.9 + (v.abs() - 0.9).tanh() * 0.1) } else { v };
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer_with(s: Sound, pcm: Vec<f32>) -> Mixer {
        let mut m = Mixer { volume: 1.0, ..Default::default() };
        m.set_sounds(HashMap::from([(s, Arc::new(pcm))]));
        m
    }

    #[test]
    fn plays_sounds_once_then_silence() {
        let mut m = mixer_with(Sound::Select, vec![0.5; 20]);
        let mut out = [0i16; 32];
        m.mix(&mut out);
        assert!(out.iter().all(|s| *s == 0), "nothing played yet");
        m.play(Sound::Select);
        m.play(Sound::Back); // no such sound loaded: ignored
        assert!(m.busy());
        m.mix(&mut out);
        assert_eq!(out[..20], [16383; 20]);
        assert!(out[20..].iter().all(|s| *s == 0));
        assert!(!m.busy(), "done");
    }

    #[test]
    fn the_same_sound_restarts_and_volume_scales() {
        let mut m = mixer_with(Sound::Scroll, vec![0.25; 8]);
        m.volume = gain_for(50);
        m.play(Sound::Scroll);
        m.play(Sound::Scroll);
        let mut out = [0i16; 8];
        m.mix(&mut out);
        assert_eq!(out[0], (0.25 * 0.25 * 32767.0) as i16, "one voice (not two added) at a quarter gain");
        assert_eq!(gain_for(0), 0.0);
        assert_eq!(gain_for(100), 1.0);
    }

    #[test]
    fn loud_mixes_are_limited() {
        let mut m = Mixer { volume: 1.0, ..Default::default() };
        m.set_sounds(Sound::ALL.iter().map(|s| (*s, Arc::new(vec![0.9; 4]))).collect());
        for s in Sound::ALL {
            m.play(s);
        }
        let mut out = [0i16; 4];
        m.mix(&mut out);
        assert!(out[0] > 29000, "near full scale, not wrapped: {}", out[0]);
    }

    #[test]
    fn music_fades_in_and_out() {
        let (tx, rx) = crossbeam_channel::bounded(4);
        let mut m = Mixer { volume: 1.0, ..Default::default() };
        m.music = Some(Music::new(rx));
        m.music_target = 1.0;
        let fade = (FADE_SECS * RATE as f32) as usize;
        std::thread::spawn(move || {
            for _ in 0..8 {
                if tx.send(vec![0.5; fade]).is_err() {
                    break;
                }
            }
        });
        let mut out = vec![0i16; fade * 2];
        let mut got = vec![];
        for _ in 0..200 {
            m.mix(&mut out);
            got.extend_from_slice(&out);
            if got.iter().rev().take(10).all(|s| *s > 9000) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let last = *got.last().unwrap();
        assert!((9000..9100).contains(&last), "full level = 0.5 × MUSIC_LEVEL: {last}");
        m.music_target = 0.0;
        m.mix(&mut out);
        m.mix(&mut out);
        assert!(m.music_silent());
        assert_eq!(*out.last().unwrap(), 0, "faded out");
    }

    #[test]
    fn music_ends_when_the_decoder_stops() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        tx.send(vec![0.1; 4]).unwrap();
        drop(tx);
        let mut m = Music::new(rx);
        assert!(m.next().is_some() && m.next().is_some());
        assert!(m.next().is_none() && m.ended);
        assert_eq!(m.played, 4);
    }
}
