//! Theme model: transitions (focus / appear animations) and the `stack` container's layout
//! enums. (Re-exported from model.rs.)
//!
//! Transitions (`transitions = { focus = { … }, appear = { … } }` on any element):
//!   * `scale`, `opacity` and `translate` describe the *other* end of the animation: for
//!     `appear` the state the element starts from (it animates to its normal look), for
//!     `disappear` the state it ends in, for `focus` the look of the focused item of a grid /
//!     carousel / rows (it animates there when the item gets the focus), for `unfocus` the
//!     timing of the way back (its values are ignored: an unfocused item has its normal look).
//!   * `translate = [x, y]` is in fractions of the screen (x of the width, y of the height),
//!     like `pos`. `scale` turns around the element's centre.
//!   * `duration_ms` (default 200), `delay_ms` (default 0), `easing` (default `ease_out`).
//!   * `appear` plays when the element starts being drawn: its view opens, or its
//!     `visible_if` / `hidden_if` lets it show again; with `replay = "select"` also each time the
//!     selection moves (Playnite themes that slide the game's info in per game).
//!   * `disappear` keeps drawing an element that `visible_if` / `hidden_if` just hid, animating
//!     it out (top-level elements; inside a `stack` a hidden child leaves at once).
//!   * Settings: reduced motion (`[controller] reduce_motion`, or the desktop's own "no
//!     animations" setting) shows every end state at once.

use serde::{Deserialize, Serialize};

/// Timing curve of a transition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    /// Quadratic, slow start.
    #[serde(alias = "ease-in", alias = "quad_in")]
    EaseIn,
    /// Quadratic, slow end (the default).
    #[default]
    #[serde(alias = "ease-out", alias = "quad_out", alias = "ease")]
    EaseOut,
    #[serde(alias = "ease-in-out", alias = "quad_in_out")]
    EaseInOut,
    #[serde(alias = "cubic-in")]
    CubicIn,
    #[serde(alias = "cubic-out")]
    CubicOut,
    #[serde(alias = "cubic-in-out", alias = "cubic")]
    CubicInOut,
}

impl Easing {
    /// Eased progress for linear progress `t` (clamped to 0..1).
    pub fn apply(self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 1.0 };
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Easing::CubicIn => t * t * t,
            Easing::CubicOut => 1.0 - (1.0 - t).powi(3),
            Easing::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// When an `appear` transition plays again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Replay {
    /// Only when the element starts being drawn.
    #[default]
    Never,
    /// Also each time the selection moves to another item.
    Select,
}

/// One animation (see the module notes for what the values mean per trigger).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translate: Option<[f32; 2]>,
    pub duration_ms: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub delay_ms: u32,
    pub easing: Easing,
    #[serde(skip_serializing_if = "is_never")]
    pub replay: Replay,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_never(r: &Replay) -> bool {
    *r == Replay::Never
}

impl Default for Transition {
    fn default() -> Self {
        Transition {
            scale: None,
            opacity: None,
            translate: None,
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::EaseOut,
            replay: Replay::Never,
        }
    }
}

impl Transition {
    pub fn secs(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }
    pub fn delay(&self) -> f64 {
        self.delay_ms as f64 / 1000.0
    }
    /// Eased progress `elapsed` seconds after the trigger (0 during the delay, 1 at the end).
    pub fn progress(&self, elapsed: f64) -> f32 {
        let d = self.secs();
        let t = elapsed - self.delay();
        if t <= 0.0 {
            return if d <= 0.0 && self.delay_ms == 0 { 1.0 } else { 0.0 };
        }
        if d <= 0.0 {
            return 1.0;
        }
        self.easing.apply((t / d) as f32)
    }
    /// The animation is over `elapsed` seconds after the trigger.
    pub fn done(&self, elapsed: f64) -> bool {
        elapsed >= self.delay() + self.secs()
    }
    /// The transform at `amount` (0 = the element's normal look, 1 = this transition's values).
    pub fn at(&self, amount: f32) -> Fx {
        let a = amount.clamp(0.0, 1.0);
        let lerp = |rest: f32, v: f32| rest + (v - rest) * a;
        Fx {
            scale: lerp(1.0, self.scale.filter(|s| s.is_finite() && *s >= 0.0).unwrap_or(1.0)),
            opacity: lerp(1.0, self.opacity.filter(|o| o.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0)),
            translate: {
                let t = self.translate.filter(|t| t.iter().all(|v| v.is_finite())).unwrap_or([0.0, 0.0]);
                [t[0] * a, t[1] * a]
            },
        }
    }
}

/// A transform applied to an element or item while it animates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx {
    pub scale: f32,
    pub opacity: f32,
    /// Fractions of the screen [x of width, y of height].
    pub translate: [f32; 2],
}

impl Fx {
    pub const NONE: Fx = Fx { scale: 1.0, opacity: 1.0, translate: [0.0, 0.0] };
    pub fn is_none(&self) -> bool {
        *self == Fx::NONE
    }
    /// Moves or resizes (more than opacity).
    pub fn moves(&self) -> bool {
        self.scale != 1.0 || self.translate != [0.0, 0.0]
    }
}

/// An element's transitions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transitions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Transition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unfocus: Option<Transition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appear: Option<Transition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disappear: Option<Transition>,
}

impl Transitions {
    /// Timing of the way back from focus: `unfocus`, else the focus transition's own.
    pub fn unfocus_timing(&self) -> Option<&Transition> {
        self.unfocus.as_ref().or(self.focus.as_ref())
    }
}

/// `stack`: which way the children flow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    #[default]
    Vertical,
    Horizontal,
}

/// `stack`: where children sit across the flow (`align`) or along it (`justify`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StackAlign {
    #[serde(alias = "left", alias = "top")]
    Start,
    Center,
    #[serde(alias = "right", alias = "bottom")]
    End,
    /// Across the flow: children without a size of their own take the stack's full width
    /// (vertical) or height (horizontal). Along the flow it means `start`.
    #[default]
    Stretch,
}

impl StackAlign {
    /// Offset of a `size` box in `room` (stretch = start).
    pub fn offset(self, room: f32, size: f32) -> f32 {
        let free = (room - size).max(0.0);
        match self {
            StackAlign::Start | StackAlign::Stretch => 0.0,
            StackAlign::Center => free / 2.0,
            StackAlign::End => free,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_run_from_zero_to_one() {
        for e in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::CubicIn,
            Easing::CubicOut,
            Easing::CubicInOut,
        ] {
            assert_eq!(e.apply(0.0), 0.0, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?}");
            let mut last = 0.0;
            for i in 1..=20 {
                let v = e.apply(i as f32 / 20.0);
                assert!(v >= last - 1e-6, "{e:?} goes back at {i}");
                last = v;
            }
            assert_eq!(e.apply(-3.0), 0.0);
            assert_eq!(e.apply(f32::NAN), 1.0);
        }
        assert!(Easing::EaseOut.apply(0.5) > 0.5 && Easing::EaseIn.apply(0.5) < 0.5);
        assert!((Easing::CubicInOut.apply(0.5) - 0.5).abs() < 1e-6);
        assert!(Easing::CubicOut.apply(0.25) > Easing::EaseOut.apply(0.25));
    }

    #[test]
    fn transition_progress_and_values() {
        let t = Transition {
            scale: Some(1.1),
            opacity: Some(0.0),
            translate: Some([0.0, 0.05]),
            duration_ms: 400,
            delay_ms: 100,
            easing: Easing::Linear,
            replay: Replay::Never,
        };
        assert_eq!(t.progress(0.05), 0.0);
        assert!((t.progress(0.3) - 0.5).abs() < 1e-6);
        assert_eq!(t.progress(0.6), 1.0);
        assert!(!t.done(0.45) && t.done(0.5));
        let half = t.at(0.5);
        assert!((half.scale - 1.05).abs() < 1e-6);
        assert!((half.opacity - 0.5).abs() < 1e-6);
        assert!((half.translate[1] - 0.025).abs() < 1e-6);
        assert!(t.at(0.0).is_none());
        // No duration: done at once.
        let instant = Transition { duration_ms: 0, ..Default::default() };
        assert_eq!(instant.progress(0.0), 1.0);
    }

    #[test]
    fn transitions_parse_with_aliases() {
        #[derive(Deserialize)]
        struct W {
            transitions: Transitions,
        }
        let w: W = toml::from_str(
            r#"transitions = { focus = { scale = 1.08, duration_ms = 150, easing = "cubic" }, appear = { opacity = 0.0, translate = [0.0, 0.02], easing = "ease-in", replay = "select" } }"#,
        )
        .unwrap();
        let f = w.transitions.focus.as_ref().unwrap();
        assert_eq!(f.easing, Easing::CubicInOut);
        assert_eq!(f.duration_ms, 150);
        let a = w.transitions.appear.as_ref().unwrap();
        assert_eq!(a.easing, Easing::EaseIn);
        assert_eq!(a.replay, Replay::Select);
        assert_eq!(a.duration_ms, 200);
        assert_eq!(w.transitions.unfocus_timing(), Some(f));
        assert_eq!(StackAlign::Center.offset(100.0, 40.0), 30.0);
        assert_eq!(StackAlign::End.offset(100.0, 40.0), 60.0);
        assert_eq!(StackAlign::Stretch.offset(100.0, 140.0), 0.0);
    }
}
