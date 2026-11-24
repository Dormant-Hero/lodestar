//! Lodestar's own soft navigation sounds, generated (for themes without sounds): short sine
//! blips with a quick fade, quiet next to theme sounds.

use super::audio::Sound;
use super::decode::RATE;

/// One tone: start / end frequency (Hz), length (s), loudness, decay speed.
struct Tone(f32, f32, f32, f32, f32);

fn render(tones: &[(f32, Tone)]) -> Vec<f32> {
    let total = tones.iter().map(|(at, t)| at + t.2).fold(0.0f32, f32::max);
    let n = (total * RATE as f32) as usize + 1;
    let mut out = vec![0.0f32; n * 2];
    for (at, Tone(f0, f1, len, amp, decay)) in tones {
        let start = (at * RATE as f32) as usize;
        let frames = (len * RATE as f32) as usize;
        let mut phase = 0.0f32;
        for i in 0..frames {
            let t = i as f32 / RATE as f32;
            let f = f0 + (f1 - f0) * (i as f32 / frames.max(1) as f32);
            phase += std::f32::consts::TAU * f / RATE as f32;
            let attack = (t / 0.002).min(1.0);
            let tail = ((frames - i) as f32 / (0.004 * RATE as f32)).min(1.0);
            let env = attack * tail * (-t * decay).exp();
            let v = phase.sin() * amp * env;
            let j = (start + i) * 2;
            if j + 1 < out.len() {
                out[j] += v;
                out[j + 1] += v;
            }
        }
    }
    out
}

/// The built-in sound for `s`.
pub fn sound(s: Sound) -> Vec<f32> {
    match s {
        Sound::Scroll => render(&[(0.0, Tone(1900.0, 1700.0, 0.018, 0.16, 160.0))]),
        Sound::SystemBrowse | Sound::QuickSysSelect => render(&[(0.0, Tone(900.0, 1300.0, 0.045, 0.16, 60.0))]),
        Sound::Select => {
            render(&[(0.0, Tone(880.0, 880.0, 0.05, 0.18, 40.0)), (0.045, Tone(1320.0, 1320.0, 0.07, 0.18, 40.0))])
        }
        Sound::Back => {
            render(&[(0.0, Tone(1320.0, 1320.0, 0.05, 0.16, 40.0)), (0.045, Tone(880.0, 880.0, 0.07, 0.16, 40.0))])
        }
        Sound::Favorite => render(&[
            (0.0, Tone(1047.0, 1047.0, 0.08, 0.15, 30.0)),
            (0.06, Tone(1319.0, 1319.0, 0.08, 0.15, 30.0)),
            (0.12, Tone(1568.0, 1568.0, 0.14, 0.15, 25.0)),
        ]),
        Sound::Launch => render(&[(0.0, Tone(420.0, 1250.0, 0.3, 0.18, 6.0))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_short_quiet_and_clean() {
        for s in Sound::ALL {
            let pcm = sound(s);
            let secs = pcm.len() as f32 / 2.0 / RATE as f32;
            assert!(secs > 0.01 && secs < 0.5, "{s:?}: {secs} s");
            let peak = pcm.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.02 && peak < 0.5, "{s:?}: peak {peak}");
            assert!(pcm[pcm.len() - 2].abs() < 0.01, "{s:?} fades out (no click)");
        }
    }
}
