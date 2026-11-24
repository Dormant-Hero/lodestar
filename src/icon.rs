//! The app icon: embedded SVG, rendered for the window icon and exported as PNGs for the
//! icon theme (`lodestar --export-icons ~/.local/share/icons/hicolor`, used by install.sh).

use std::path::Path;

const SVG: &[u8] = include_bytes!("../packaging/lodestar.svg");

/// Render the icon at `size`x`size` as straight (unpremultiplied) RGBA.
pub fn render(size: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(SVG, &usvg::Options::default()).ok()?;
    let s = tree.size();
    let scale = size as f32 / s.width().max(s.height());
    let mut pm = tiny_skia::Pixmap::new(size, size)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    let mut px = pm.take();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(px)
}

pub fn window_icon() -> Option<egui::IconData> {
    let size = 128;
    render(size).map(|rgba| egui::IconData { rgba, width: size, height: size })
}

/// Write `<hicolor>/<N>x<N>/apps/lodestar.png` for the usual sizes plus the scalable SVG.
pub fn export(hicolor: &Path) -> anyhow::Result<()> {
    for size in [16u32, 24, 32, 48, 64, 96, 128, 192, 256, 512] {
        let dir = hicolor.join(format!("{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        let rgba = render(size).ok_or_else(|| anyhow::anyhow!("icon render failed"))?;
        image::save_buffer(dir.join("lodestar.png"), &rgba, size, size, image::ColorType::Rgba8)?;
    }
    let dir = hicolor.join("scalable/apps");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("lodestar.svg"), SVG)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders() {
        let px = super::render(64).unwrap();
        assert_eq!(px.len(), 64 * 64 * 4);
        assert!(px.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
    }
}
