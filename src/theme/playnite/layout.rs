//! Playnite's side of the box layout (`theme::layout`): the screen its Fullscreen views are
//! designed for, and grid lengths, alignments and stretches read from their XAML text.

use crate::theme::layout::{Al, Len, Scale};

/// Playnite Fullscreen's design size.
pub(crate) const SCREEN: [f32; 2] = [1920.0, 1080.0];

impl Len {
    /// `Auto`, `*`, `2*`, `120` (WPF GridLength text).
    pub fn parse(s: &str) -> Len {
        let t = s.trim();
        if t.eq_ignore_ascii_case("auto") {
            return Len::Auto;
        }
        if let Some(w) = t.strip_suffix('*') {
            let w = if w.trim().is_empty() { 1.0 } else { w.trim().parse::<f32>().unwrap_or(1.0) };
            return Len::Star(if w.is_finite() && w > 0.0 { w } else { 1.0 });
        }
        match t.parse::<f32>() {
            Ok(v) if v.is_finite() && v >= 0.0 => Len::Px(v),
            _ => Len::Star(1.0),
        }
    }
}

impl Al {
    /// `Left`, `Center`, `Bottom`, … (WPF HorizontalAlignment / VerticalAlignment text).
    pub fn parse(s: &str) -> Option<Al> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "stretch" => Al::Stretch,
            "left" | "top" => Al::Start,
            "center" => Al::Center,
            "right" | "bottom" => Al::End,
            _ => return None,
        })
    }
}

impl Scale {
    /// `None`, `Uniform`, `UniformToFill`, `Fill` (WPF Stretch text).
    pub fn parse(s: &str) -> Option<Scale> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "none" => Scale::None,
            "uniform" => Scale::Uniform,
            "uniformtofill" => Scale::UniformToFill,
            "fill" => Scale::Fill,
            _ => return None,
        })
    }
}
