//! Text measured in Lodestar's fonts (egui's layout) for the WPF layout pass.

use super::*;

/// Text measured with the fonts Lodestar will draw it in (egui's text layout, the same line
/// height as the theme renderer): the theme's own font when it brings one, else Titillium Web
/// (Playnite's), else egui's default.
pub(crate) struct FontMeasurer {
    fonts: egui::epaint::text::Fonts,
    bold: bool,
}

impl FontMeasurer {
    pub fn new(regular: Option<egui::FontData>, bold: Option<egui::FontData>) -> FontMeasurer {
        use egui::{FontData, FontDefinitions, FontFamily};
        use std::sync::Arc;
        let bundled = |n: &str| crate::theme::fonts::bundled::data(n).map(FontData::from_static);
        let regular = regular.or_else(|| bundled("titillium-web-light"));
        let bold_data = bold.or_else(|| bundled("titillium-web-bold"));
        let mut defs = FontDefinitions::default();
        let mut has_bold = false;
        if let Some(r) = regular {
            defs.font_data.insert("pn_regular".into(), Arc::new(r));
            if let Some(list) = defs.families.get_mut(&FontFamily::Proportional) {
                list.insert(0, "pn_regular".into());
            }
        }
        if let Some(b) = bold_data {
            defs.font_data.insert("pn_bold".into(), Arc::new(b));
            let mut chain = vec!["pn_bold".to_string()];
            chain.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
            defs.families.insert(FontFamily::Name("pn_bold".into()), chain);
            has_bold = true;
        }
        FontMeasurer {
            fonts: egui::epaint::text::Fonts::new(egui::epaint::text::TextOptions::default(), defs),
            bold: has_bold,
        }
    }
}

impl Measurer for FontMeasurer {
    fn measure(&mut self, t: &TextSpec, max_w: f32) -> [f32; 2] {
        let line = t.size * 1.344;
        if t.text.is_empty() {
            return [0.0, line];
        }
        let family =
            if t.bold && self.bold { egui::FontFamily::Name("pn_bold".into()) } else { egui::FontFamily::Proportional };
        let wrap = if t.wrap && max_w.is_finite() { max_w.max(1.0) } else { f32::INFINITY };
        let g = self.fonts.with_pixels_per_point(1.0).layout(
            t.text.clone(),
            egui::FontId::new(t.size.max(1.0), family),
            egui::Color32::WHITE,
            wrap,
        );
        let mut rows = g.rows.len().max(1) as f32;
        if let Some(m) = t.max_lines {
            rows = rows.min(m.max(1) as f32);
        }
        [g.size().x.ceil(), rows * line]
    }
}
