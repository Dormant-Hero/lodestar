//! Drawing a theme video (an image element's `video`): the newest decoded frame, placed like
//! the element's picture. False = nothing to show yet, so the caller draws the still picture.

use super::*;

impl Renderer {
    pub(super) fn draw_video(
        &mut self,
        f: &mut Frame,
        common: &Common,
        video: &str,
        fit: Fit,
        tint: Option<Color>,
        corner_radius: f32,
    ) -> bool {
        if !self.opts.videos || !crate::media::videos_available() {
            return false;
        }
        let Some(path) = self.pick_video(f, video) else {
            return false;
        };
        let (sw, sh) = (f.sw(), f.sh());
        // Auto-sized boxes take the screen's size as the decode budget.
        let px = [
            if common.size[0] > 0.0 { common.size[0] * sw } else { sw } * f.ppp,
            if common.size[1] > 0.0 { common.size[1] * sh } else { sh } * f.ppp,
        ];
        let ctx = f.painter.ctx().clone();
        let Some(tex) = self.videos.frame(&ctx, &path, px) else {
            return false;
        };
        let natural = tex.size_vec2() / f.ppp;
        let rect = f.bleed(Self::elem_rect(f, common, natural));
        let tint = fade(tint.map(|c| c.to_egui()).unwrap_or(Color32::WHITE), common.opacity);
        paint_image_aligned(
            &f.painter,
            rect,
            &tex,
            fit,
            tint,
            corner_radius * sh,
            common.rotation,
            Vec2::from(common.origin),
            None,
        );
        true
    }

    /// The first existing file among the video's candidates.
    fn pick_video(&mut self, f: &Frame, video: &str) -> Option<PathBuf> {
        let mut cands = std::mem::take(&mut self.candidates);
        cands.clear();
        f.base.path_candidates(video, &mut cands);
        let found = cands.iter().filter_map(|c| Self::to_path(f.theme, c)).find(|p| self.exists(p));
        self.candidates = cands;
        found
    }
}
