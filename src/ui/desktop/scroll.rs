//! Keeping the selected game in view in the virtualised grid and lists.

/// Keeps a virtualised list's selection in view by setting its scroll offset directly (see
/// `grid_view`): remembers last frame's viewport height and offset.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScrollKeep {
    vp_h: Option<f32>,
    offset: f32,
    /// The offset asked for this frame (it can fall short when the content just grew).
    wanted: Option<f32>,
    /// Content height as of last frame.
    content_h: Option<f32>,
    /// This frame's offset stopped at last frame's end (ask again next frame).
    short: bool,
    /// Frames spent asking again (capped: never a repaint loop).
    retries: u8,
}

impl ScrollKeep {
    /// `area` scrolled so content range `top..bottom` is visible (when `range` is given).
    pub fn apply(
        &mut self,
        area: egui::ScrollArea,
        avail_h: f32,
        range: Option<(f32, f32)>,
        content_h: f32,
    ) -> egui::ScrollArea {
        self.wanted = None;
        let prev_h = self.content_h.replace(content_h).unwrap_or(content_h);
        let Some((top, bottom)) = range else {
            return area;
        };
        let vp_h = self.vp_h.unwrap_or(avail_h);
        match offset_to_show(self.offset, vp_h, top, bottom, content_h) {
            Some(off) => {
                // Never past last frame's end (the scroll area would spring back from there
                // with a drifting velocity); `update` asks again when that falls short.
                let limit = (prev_h - vp_h).max(0.0);
                self.short = off > limit + 0.5;
                let off = off.min(limit);
                self.wanted = Some(off);
                area.vertical_scroll_offset(off)
            }
            None => area,
        }
    }

    /// Record this frame's result; true when the viewport height changed or the offset asked
    /// for wasn't reached (re-scroll then).
    pub fn update(&mut self, offset: f32, vp_h: f32) -> bool {
        let asked = self.wanted.is_some();
        let short = self.wanted.take().is_some_and(|w| (w - offset).abs() > 1.0) || std::mem::take(&mut self.short);
        let again = short && self.retries < 8;
        if again {
            self.retries += 1;
        } else if asked && !short {
            self.retries = 0;
        }
        let changed = self.vp_h.is_some_and(|h| (h - vp_h).abs() > 0.5) || again;
        self.vp_h = Some(vp_h);
        self.offset = offset;
        changed
    }
}

/// Scroll offset that brings the content range `top..bottom` into a viewport of height `vp_h`
/// currently scrolled to `cur`, moving as little as possible (None = already visible).
pub(crate) fn offset_to_show(cur: f32, vp_h: f32, top: f32, bottom: f32, content_h: f32) -> Option<f32> {
    let max = (content_h - vp_h).max(0.0);
    let want = if top < cur || bottom - top > vp_h {
        top
    } else if bottom > cur + vp_h {
        bottom - vp_h
    } else {
        return None;
    };
    let want = want.clamp(0.0, max);
    ((want - cur).abs() > 0.5).then_some(want)
}
