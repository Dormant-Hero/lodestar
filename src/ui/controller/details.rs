//! Built-in game details screen (used when the theme has no `details` view), drawn with the
//! theme's palette and fonts: hero, logo/title, a big Play button, stats, description, cover.

use super::look::Look;
use crate::achievements::State;
use crate::images::{ImageCache, ImgSize};
use crate::model::*;
use crate::theme::render::draw::*;
use crate::theme::{Fit, TextStyle};
use egui::{Color32, Rect, Stroke, StrokeKind, pos2, vec2};

pub const BUTTONS: usize = 4;

pub struct DetailsState {
    pub game: GameId,
    /// 0 = Play, 1 = Favourite, 2 = Artwork (the viewer), 3 = Options.
    pub focus: usize,
    /// Description scroll target (points) and animated value.
    pub scroll: f32,
    scroll_anim: f32,
    focus_anim: f32,
    pub max_scroll: f32,
}

impl DetailsState {
    pub fn new(game: GameId) -> DetailsState {
        DetailsState { game, focus: 0, scroll: 0.0, scroll_anim: 0.0, focus_anim: 0.0, max_scroll: 0.0 }
    }
    pub fn scroll_by(&mut self, d: f32) {
        self.scroll = (self.scroll + d).clamp(0.0, self.max_scroll.max(0.0));
    }
}

#[derive(Clone, Copy)]
enum StatIcon {
    Gamepad(ControllerSupport),
    Deck(DeckRating),
    /// The value is drawn as a chip in this ProtonDB tier colour.
    Chip(Color32),
}

/// What the big primary button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Primary {
    Play,
    Stop,
    /// Launched but not up yet (a store client starting, signing in …): the same button
    /// cancels the launch.
    Cancel,
    /// Not installed, and its store / add-on can install it.
    Install,
    /// No longer in the account's Steam library: A opens the game in Steam (steam_owned.rs).
    OpenInSteam,
}

impl Primary {
    pub fn for_game(game: &Game, running_this: bool, starting: bool, can_install: bool) -> Primary {
        if running_this && starting {
            Primary::Cancel
        } else if running_this {
            Primary::Stop
        } else if !game.installed && can_install {
            Primary::Install
        } else {
            Primary::Play
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Primary::Play => "Play",
            Primary::Stop => "Quit game",
            Primary::Cancel => "Cancel launch",
            Primary::Install => "Install",
            Primary::OpenInSteam => "Open in Steam",
        }
    }

    /// Quits or cancels the game (drawn as a danger button).
    pub fn stops(self) -> bool {
        matches!(self, Primary::Stop | Primary::Cancel)
    }
}

/// Chip colours for a ProtonDB tier: (fill, text).
pub fn protondb_colors(tier: &str) -> Option<(Color32, Color32)> {
    let [r, g, b] = crate::metadata::protondb::tier_rgb(tier)?;
    let fill = Color32::from_rgb(r, g, b);
    // Dark text on the light tiers, white on red / grey.
    Some((fill, protondb_text_on(fill)))
}

#[derive(Default)]
pub struct DetailsOut {
    pub animating: bool,
    pub clicked: Option<usize>,
}

fn approach(v: &mut f32, target: f32, dt: f32, rate: f32, out: &mut bool) {
    let d = target - *v;
    if d.abs() < 0.001 {
        *v = target;
    } else {
        *v += d * (1.0 - (-dt * rate).exp());
        *out = true;
    }
}

fn tex(images: &mut ImageCache, p: &Option<std::path::PathBuf>, size: ImgSize) -> Option<egui::TextureHandle> {
    p.as_deref().and_then(|p| images.get(p, size))
}

/// Load the pictures `draw` shows for `game` ahead (the game A or PageDown opens next).
pub fn prefetch(images: &mut ImageCache, game: &Game) {
    let a = &game.art;
    if let Some(p) = a.hero.as_deref().or(a.banner.as_deref()).or(a.cover.as_deref()) {
        images.prefetch(p, ImgSize::Thumb);
    }
    for (p, size) in [(&a.hero, ImgSize::Full), (&a.logo, ImgSize::Medium), (&a.cover, ImgSize::Medium)] {
        if let Some(p) = p {
            images.prefetch(p, size);
        }
    }
}

/// Text shadow under the title, meta line and stats: they can sit on bright hero art.
const SHADOW: Option<Color32> = Some(Color32::from_black_alpha(130));

/// What every part of the page draws with: the painter, the look and the screen.
struct Page<'a> {
    p: &'a egui::Painter,
    look: &'a Look,
    screen: Rect,
    sw: f32,
    sh: f32,
}

impl Page<'_> {
    /// A point at a fraction of the screen's width and height.
    fn at(&self, x: f32, y: f32) -> egui::Pos2 {
        self.screen.min + vec2(x * self.sw, y * self.sh)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &egui::Ui,
    screen: Rect,
    look: &Look,
    images: &mut ImageCache,
    game: &Game,
    primary: Primary,
    s: &mut DetailsState,
    hints: &[Hint],
    dt: f32,
    opacity: f32,
    achievements: Option<&crate::achievements::State>,
) -> DetailsOut {
    let mut out = DetailsOut::default();
    let mut p = ui.painter_at(screen);
    p.multiply_opacity(opacity);
    let page = Page { p: &p, look, screen, sw: look.sw, sh: look.sh };
    p.rect_filled(screen, 0.0, look.bg);
    paint_backdrop(&page, images, game);
    paint_title(&page, images, game);
    paint_meta_line(&page, game);

    let mut moving = false;
    approach(&mut s.focus_anim, s.focus as f32, dt, 18.0, &mut moving);
    let rects = paint_buttons(&page, game, primary, s);
    if primary == Primary::OpenInSteam {
        paint_steam_library_note(&page, rects[0].bottom());
    }
    if let Some((pos, true)) = ui.input(|i| i.pointer.hover_pos().map(|p| (p, i.pointer.primary_clicked()))) {
        out.clicked = rects.iter().position(|r| r.contains(pos));
    }

    paint_stats(&page, &stats(game));
    // Achievements (a column between the description and the cover) once there is something.
    let ach = achievements.filter(|a| matches!(a, State::Ready(_) | State::Hint(_)));
    let desc_right = match ach {
        Some(a) => {
            let r = achievements_rect(&page);
            paint_achievements(&page, images, a, r);
            r.left() - 0.03 * page.sw
        }
        None => page.at(0.58, 0.0).x,
    };
    paint_description(&page, game, s, dt, &mut moving, desc_right);
    paint_cover(&page, images, game);
    paint_help_bar(&page, hints);

    out.animating = moving;
    out
}

/// Backdrop: blurred hero over the whole screen, crisp hero on top fading into the background.
fn paint_backdrop(page: &Page, images: &mut ImageCache, game: &Game) {
    let Page { p, look, screen, sw, sh } = *page;
    let hero_src = game.art.hero.clone().or_else(|| game.art.banner.clone()).or_else(|| game.art.cover.clone());
    // The user's background placement (Arrange mode) crops every layer of it.
    let crop = game.edits.background_placement;
    if let Some(t) = tex(images, &hero_src, ImgSize::Thumb) {
        paint_blurred(p, screen, &t, Fit::Cover, fade(Color32::WHITE, 0.45), look.h(0.025), crop);
    }
    let hero_rect = Rect::from_min_size(screen.min, vec2(sw, 0.72 * sh));
    if hero_src.is_none() {
        // No artwork at all: a wash in its console's / store's colour with the console's
        // outline, as the home page of the Fullscreen look does, instead of a blank page.
        let tint = crate::theme::render::draw::placeholder_tint(game);
        gradient_rect(p, hero_rect, 0.0, [fade(tint, 0.5), fade(tint, 0.0)], false);
        let side = 0.3 * sh;
        crate::ui::platform_icons::paint(
            p,
            Rect::from_center_size(page.at(0.76, 0.33), vec2(side, side)),
            crate::ui::platform_icons::game_shape(game),
            fade(Color32::WHITE, 0.1),
        );
    }
    // Darken the blurred layer so text stays readable, then the crisp hero fading out at the bottom.
    p.rect_filled(screen, 0.0, fade(look.bg, 0.35));
    if let Some(t) = tex(images, &game.art.hero, ImgSize::Full) {
        paint_image_masked(
            p,
            hero_rect,
            &t,
            Fit::Cover,
            fade(Color32::WHITE, 0.95),
            [0.0, 0.0, 0.0, 0.5],
            vec2(0.5, 0.5),
            crop,
        );
    }
    gradient_rect(
        p,
        Rect::from_min_size(screen.min, vec2(0.7 * sw, sh)),
        0.0,
        [fade(look.bg, 0.94), fade(look.bg, 0.0)],
        true,
    );
    gradient_rect(
        p,
        Rect::from_min_max(pos2(screen.left(), screen.top() + 0.5 * sh), screen.max),
        0.0,
        [fade(look.bg, 0.0), fade(look.bg, 0.6)],
        false,
    );
}

/// Logo or title, anchored to the bottom-left of its box.
fn paint_title(page: &Page, images: &mut ImageCache, game: &Game) {
    let Page { p, look, sw, .. } = *page;
    let logo_box = Rect::from_min_max(page.at(0.06, 0.1), page.at(0.46, 0.33));
    match tex(images, &game.art.logo, ImgSize::Medium) {
        Some(t) => {
            paint_image_aligned(p, logo_box, &t, Fit::Contain, Color32::WHITE, 0.0, 0.0, vec2(0.0, 1.0), None);
        }
        None => {
            let style = TextStyle { line_spacing: 1.05, ..Default::default() };
            let g = layout(
                p,
                game.display_name(),
                look.title.clone(),
                look.h(0.068),
                Color32::WHITE,
                &style,
                Some(0.55 * sw),
                Some(2),
            );
            paint_galley(p, pos2(logo_box.left(), logo_box.bottom() - g.size().y), &g, Color32::WHITE, SHADOW, 0.0);
        }
    }
}

/// Source · developer · year · genres, led by a "NOT INSTALLED" pill when it isn't.
fn paint_meta_line(page: &Page, game: &Game) {
    let Page { p, look, sw, .. } = *page;
    let mut meta: Vec<String> = vec![super::emu::source_label(game)];
    if let Some(d) = &game.meta.developer {
        meta.push(d.clone());
    }
    if let Some(y) = game.meta.release_year {
        meta.push(y.to_string());
    }
    if !game.meta.genres.is_empty() {
        meta.push(game.meta.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
    }
    // "Not installed" leads the meta line as a pill, so it reads before anything else.
    let meta_pos = page.at(0.06, 0.352);
    let mut meta_x = meta_pos;
    let pill = (!game.installed).then(|| {
        let g = p.layout_no_wrap("NOT INSTALLED".into(), look.bold(0.0165), look.text);
        let pad = vec2(look.h(0.012), look.h(0.005));
        meta_x.x += g.size().x + pad.x * 2.0 + look.h(0.014);
        (g, pad)
    });
    let mg = layout_line(p, &meta.join("  ·  "), look.medium(0.022), look.dim, 0.6 * sw - (meta_x.x - meta_pos.x));
    if let Some((g, pad)) = pill {
        let size = g.size() + pad * 2.0;
        let chip = Rect::from_min_size(pos2(meta_pos.x, meta_pos.y + (mg.size().y - size.y) / 2.0), size);
        p.rect_filled(chip, cr(chip.height() / 2.0), Color32::from_black_alpha(150));
        p.rect_stroke(
            chip,
            cr(chip.height() / 2.0),
            Stroke::new(look.h(0.0016), fade(look.text, 0.6)),
            StrokeKind::Inside,
        );
        p.galley(chip.min + pad, g, look.text);
    }
    paint_galley(p, meta_x, &mg, look.dim, SHADOW, 0.0);
}

/// Play / Favourite / Artwork / Options with the focus ring gliding between them and the
/// focused small button's name beside them. Returns the buttons' rects (for clicks).
fn paint_buttons(page: &Page, game: &Game, primary: Primary, s: &DetailsState) -> [Rect; BUTTONS] {
    let Page { p, look, .. } = *page;
    let by = 0.405;
    let bh = look.h(0.074);
    let play_w = look.h(0.3);
    let gap = look.h(0.018);
    let mut rects = [Rect::NOTHING; BUTTONS];
    rects[0] = Rect::from_min_size(page.at(0.06, by), vec2(play_w, bh));
    rects[1] = Rect::from_min_size(pos2(rects[0].right() + gap, rects[0].top()), vec2(bh, bh));
    rects[2] = Rect::from_min_size(pos2(rects[1].right() + gap, rects[0].top()), vec2(bh, bh));
    rects[3] = Rect::from_min_size(pos2(rects[2].right() + gap, rects[0].top()), vec2(bh, bh));
    // Focus ring glides between buttons.
    let fi = s.focus_anim.clamp(0.0, (BUTTONS - 1) as f32);
    let (i0, i1) = (fi.floor() as usize, (fi.ceil() as usize).min(BUTTONS - 1));
    let ft = fi - i0 as f32;
    let lerp_rect =
        |a: Rect, b: Rect, t: f32| Rect::from_min_max(a.min + (b.min - a.min) * t, a.max + (b.max - a.max) * t);
    let ring = lerp_rect(rects[i0], rects[i1], ft).expand(look.h(0.006));
    for (i, r) in rects.iter().enumerate() {
        let focused = i == s.focus;
        let (fill, fg) = match i {
            0 if primary.stops() => (look.danger, Color32::WHITE),
            0 => (look.accent, look.accent_text),
            _ if focused => (fade(Color32::WHITE, 0.22), look.text),
            _ => (fade(Color32::WHITE, 0.1), look.text),
        };
        if i == 0 {
            drop_shadow(p, *r, bh / 2.0, 0.6);
        }
        p.rect_filled(*r, cr(bh / 2.0), fill);
        match i {
            0 => paint_primary_label(page, *r, bh, game, primary, fg),
            1 => {
                let col = if game.favorite { look.favorite } else { fade(look.text, 0.85) };
                star(p, r.center(), bh * 0.27, col);
                if !game.favorite {
                    star(p, r.center(), bh * 0.17, fill);
                }
            }
            // Dimmed when there is nothing to view (A then says so).
            2 => picture_glyph(p, r.center(), bh * 0.5, if game.art.is_empty() { fade(fg, 0.35) } else { fg }),
            _ => {
                for k in -1..=1 {
                    p.circle_filled(r.center() + vec2(k as f32 * bh * 0.17, 0.0), bh * 0.05, fg);
                }
            }
        }
    }
    p.rect_stroke(ring, cr(ring.height() / 2.0), Stroke::new(look.h(0.0035), look.selection), StrokeKind::Outside);
    let names = [
        match primary {
            Primary::Install if game.source == crate::model::Source::Romm => "Download",
            _ => primary.label(),
        },
        if game.favorite { "Unfavourite" } else { "Favourite" },
        if game.art.is_empty() { "No artwork yet" } else { "Artwork" },
        "Options",
    ];
    if s.focus > 0 {
        let g = p.layout_no_wrap(names[s.focus].into(), look.medium(0.019), look.dim);
        let last = rects[BUTTONS - 1];
        p.galley(pos2(last.right() + gap * 1.5, last.center().y - g.size().y / 2.0), g, look.dim);
    }
    rects
}

/// The primary button's glyph (play, stop or download) and label, centred in `r`.
fn paint_primary_label(page: &Page, r: Rect, bh: f32, game: &Game, primary: Primary, fg: Color32) {
    let Page { p, look, .. } = *page;
    let label = if primary == Primary::Install && game.source == crate::model::Source::Romm {
        "Download"
    } else {
        primary.label()
    };
    let g = p.layout_no_wrap(label.into(), look.bold(0.03), fg);
    let icon_w = bh * 0.32;
    let total = icon_w + bh * 0.22 + g.size().x;
    let x0 = r.center().x - total / 2.0;
    let c = pos2(x0 + icon_w / 2.0, r.center().y);
    if primary.stops() {
        p.rect_filled(Rect::from_center_size(c, vec2(icon_w * 0.85, icon_w * 0.85)), cr(2.0), fg);
    } else if primary == Primary::Install {
        download_glyph(p, c, icon_w, fg);
    } else if primary == Primary::OpenInSteam {
        open_glyph(p, c, icon_w, fg);
    } else {
        let h = icon_w;
        let pts = vec![c + vec2(-h * 0.4, -h * 0.5), c + vec2(h * 0.55, 0.0), c + vec2(-h * 0.4, h * 0.5)];
        p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE));
    }
    p.galley(pos2(x0 + icon_w + bh * 0.22, r.center().y - g.size().y / 2.0), g, fg);
}

/// The stats row, most important first: (label, value, drawn icon).
fn stats(game: &Game) -> Vec<(&'static str, String, Option<StatIcon>)> {
    let mut stats: Vec<(&str, String, Option<StatIcon>)> = vec![
        (
            "Play time",
            if game.playtime_secs > 0 { format_playtime(game.playtime_secs) } else { "None yet".into() },
            None,
        ),
        ("Last played", format_last_played(game.last_played), None),
    ];
    // The user's own verdicts, only when set.
    if let Some(c) = game.completion() {
        stats.push(("Completion", c.label().into(), None));
    }
    if let Some(sc) = game.edits.user_score {
        stats.push(("Your score", format!("{}/100", sc.min(100)), None));
    }
    if let Some(c) = game.controller() {
        let v = match c {
            ControllerSupport::Full => "Full support",
            ControllerSupport::Partial => "Partial support",
            ControllerSupport::None => "No support",
        };
        stats.push(("Controller", v.into(), Some(StatIcon::Gamepad(c))));
    }
    if let Some(d) = game.meta.deck {
        let v = match d {
            DeckRating::Verified => "Verified",
            DeckRating::Playable => "Playable",
            DeckRating::Unsupported => "Unsupported",
        };
        stats.push(("Steam Deck", v.into(), Some(StatIcon::Deck(d))));
    }
    if let Some(t) = game.meta.protondb.as_deref()
        && let (Some(label), Some((fill, _))) = (crate::metadata::protondb::tier_label(t), protondb_colors(t))
    {
        stats.push(("ProtonDB", label.into(), Some(StatIcon::Chip(fill))));
    }
    if game.play_count > 0 {
        stats.push(("Launched", format!("{}×", game.play_count), None));
    }
    if let Some(b) = game.size_bytes.filter(|_| game.installed) {
        stats.push(("Size", crate::theme::render::bind::format_size(b), None));
    } else if game.installed {
        stats.push(("Status", "Installed".into(), None));
    }
    // Not installed: the pill on the meta line says so.
    stats
}

/// The stats row: it stops before the cover on the right, so the last stats drop out on narrow
/// screens. Feature stats (controller support, Steam Deck rating) get a drawn icon.
fn paint_stats(page: &Page, stats: &[(&str, String, Option<StatIcon>)]) {
    let Page { p, look, screen, sw, sh } = *page;
    let stats_right = screen.right() - 0.07 * sw - 0.5 * sh * 0.75 - look.h(0.04);
    let mut x = 0.06 * sw;
    for (label, value, icon) in stats {
        let lg = p.layout_no_wrap(label.to_uppercase(), look.bold(0.0155), look.dim);
        let chip = match icon {
            Some(StatIcon::Chip(fill)) => Some(*fill),
            _ => None,
        };
        let vg = match chip {
            // Chip text: smaller and bold, coloured for contrast against the tier colour.
            Some(fill) => {
                let text = protondb_text_on(fill);
                p.layout_no_wrap(value.clone(), look.bold(0.02), text)
            }
            None => p.layout_no_wrap(value.clone(), look.medium(0.026), look.text),
        };
        let vh = vg.size().y;
        let glyph_w = match icon {
            Some(StatIcon::Gamepad(_)) => vh * 0.95,
            Some(StatIcon::Deck(_)) => vh * 0.72,
            Some(StatIcon::Chip(_)) | None => 0.0,
        };
        let icon_w =
            if matches!(icon, Some(StatIcon::Gamepad(_) | StatIcon::Deck(_))) { glyph_w + look.h(0.01) } else { 0.0 };
        let chip_pad = vec2(look.h(0.012), look.h(0.004));
        let value_w = match chip {
            Some(_) => vg.size().x + chip_pad.x * 2.0,
            None => vg.size().x,
        };
        let w = lg.size().x.max(icon_w + value_w);
        if x > 0.06 * sw && screen.left() + x + w > stats_right {
            break;
        }
        let top = screen.top() + 0.53 * sh;
        paint_galley(p, pos2(screen.left() + x, top), &lg, look.dim, SHADOW, 0.0);
        let vy = top + lg.size().y + look.h(0.006);
        if let Some(icon) = icon {
            let c = pos2(screen.left() + x + glyph_w / 2.0, vy + vh * 0.52);
            match *icon {
                StatIcon::Gamepad(cs) => gamepad_glyph(
                    p,
                    c,
                    glyph_w,
                    if cs == ControllerSupport::None { look.dim } else { look.text },
                    look.bg,
                ),
                StatIcon::Deck(d) => deck_glyph(p, c, glyph_w / 2.0, d, look.bg),
                StatIcon::Chip(_) => {}
            }
        }
        if let Some(fill) = chip {
            // Same height as the other stats' values, vertically centred on them.
            let value_h = p.layout_no_wrap("0".into(), look.medium(0.026), look.text).size().y;
            let ch = vh + chip_pad.y * 2.0;
            let r = Rect::from_min_size(
                pos2(screen.left() + x, vy + (value_h - ch) / 2.0 + look.h(0.002)),
                vec2(value_w, ch),
            );
            p.rect_filled(r, cr(ch / 2.0), fill);
            let text = protondb_text_on(fill);
            p.galley(r.min + chip_pad, vg, text);
        } else {
            paint_galley(p, pos2(screen.left() + x + icon_w, vy), &vg, look.text, SHADOW, 0.0);
        }
        x += w + look.h(0.06);
    }
}

/// The description, scrolled with up / down, fading out at a cut edge.
fn paint_description(page: &Page, game: &Game, s: &mut DetailsState, dt: f32, moving: &mut bool, right: f32) {
    let Page { p, look, .. } = *page;
    let desc_rect = Rect::from_min_max(page.at(0.06, 0.645), pos2(right, page.at(0.0, 0.885).y));
    let Some(d) = game.meta.description.as_deref().map(str::trim).filter(|d| !d.is_empty()) else {
        s.max_scroll = 0.0;
        let g = p.layout_no_wrap("No description available.".into(), look.font(0.021), fade(look.dim, 0.8));
        p.galley(desc_rect.min, g, look.dim);
        return;
    };
    // Wikipedia's text is CC BY-SA: credited wherever it is shown.
    let credited;
    let d = if game.meta.wikipedia.is_some() {
        credited = format!("{d}{}", crate::metadata::wikipedia::CREDIT_LINE);
        credited.as_str()
    } else {
        d
    };
    let style = TextStyle { line_spacing: 1.35, ..Default::default() };
    let g = layout(p, d, look.regular.clone(), look.h(0.021), look.text, &style, Some(desc_rect.width()), None);
    s.max_scroll = (g.size().y - desc_rect.height()).max(0.0);
    s.scroll = s.scroll.clamp(0.0, s.max_scroll);
    approach(&mut s.scroll_anim, s.scroll, dt, 14.0, moving);
    let dp = p.with_clip_rect(desc_rect);
    dp.galley(desc_rect.min - vec2(0.0, s.scroll_anim), g, look.text);
    let fade_h = look.h(0.04);
    if s.scroll_anim < s.max_scroll - 1.0 {
        let b = Rect::from_min_max(pos2(desc_rect.left(), desc_rect.bottom() - fade_h), desc_rect.max);
        gradient_rect(&dp, b.expand2(vec2(2.0, 0.0)), 0.0, [fade(look.bg, 0.0), fade(look.bg, 0.92)], false);
    }
    if s.scroll_anim > 1.0 {
        let t = Rect::from_min_max(desc_rect.min, pos2(desc_rect.right(), desc_rect.top() + fade_h));
        gradient_rect(&dp, t.expand2(vec2(2.0, 0.0)), 0.0, [fade(look.bg, 0.92), fade(look.bg, 0.0)], false);
    }
}

/// Where the achievements column goes: left of the cover, as low as the description.
fn achievements_rect(page: &Page) -> Rect {
    let Page { look, screen, sw, sh, .. } = *page;
    let right = screen.right() - 0.07 * sw - 0.5 * sh * 0.75 - look.h(0.04);
    let w = look.h(0.42).min(0.3 * sw);
    // A little lower than the description: the button legend starts at 0.925.
    Rect::from_min_max(pos2(right - w, screen.top() + 0.645 * sh), pos2(right, screen.top() + 0.905 * sh))
}

/// Achievements, compactly: "ACHIEVEMENTS · STEAM", "12 of 40" and the share, a bar, the latest
/// unlocks as far as they fit; a hint is one wrapped line.
fn paint_achievements(page: &Page, images: &mut ImageCache, state: &State, r: Rect) {
    use crate::ui::achievements as ach;
    let Page { p, look, .. } = *page;
    let p = p.with_clip_rect(r.expand(2.0));
    let progress = match state {
        State::Ready(pr) => Some(pr),
        _ => None,
    };
    let head = match progress {
        Some(pr) => format!("ACHIEVEMENTS · {}", pr.service.label().to_uppercase()),
        None => "ACHIEVEMENTS".to_string(),
    };
    let lg = p.layout_no_wrap(head, look.bold(0.0155), look.dim);
    paint_galley(&p, r.min, &lg, look.dim, SHADOW, 0.0);
    let mut y = r.top() + lg.size().y + look.h(0.008);
    let Some(pr) = progress else {
        if let State::Hint(text) = state {
            let style = TextStyle { line_spacing: 1.25, ..Default::default() };
            let g = layout(&p, text, look.regular.clone(), look.h(0.019), look.dim, &style, Some(r.width()), None);
            p.galley(pos2(r.left(), y), g, look.dim);
        }
        return;
    };
    let count = p.layout_no_wrap(ach::count_line(pr), look.medium(0.026), look.text);
    paint_galley(&p, pos2(r.left(), y), &count, look.text, SHADOW, 0.0);
    if pr.synced && pr.total > 0 {
        let pct = p.layout_no_wrap(ach::percent_label(pr), look.font(0.019), look.dim);
        let at = pos2(r.right() - pct.size().x, y + count.size().y - pct.size().y - look.h(0.002));
        paint_galley(&p, at, &pct, look.dim, SHADOW, 0.0);
    }
    y += count.size().y + look.h(0.008);
    if pr.synced {
        let bar = Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), look.h(0.007)));
        p.rect_filled(bar, cr(bar.height() / 2.0), fade(look.text, 0.16));
        let f = ach::fraction(pr);
        if f > 0.0 {
            let fill = Rect::from_min_size(bar.min, vec2((bar.width() * f).max(bar.height()), bar.height()));
            p.rect_filled(fill, cr(bar.height() / 2.0), look.accent);
        }
        y += bar.height() + look.h(0.012);
    }
    if let Some(line) = ach::hardcore_line(pr).or_else(|| pr.note.clone()) {
        let g = p.layout_no_wrap(line, look.font(0.0165), look.dim);
        paint_galley(&p, pos2(r.left(), y), &g, look.dim, SHADOW, 0.0);
        y += g.size().y + look.h(0.01);
    }
    let icon = look.h(0.036);
    for a in pr.recent().into_iter().take(3) {
        if y + icon > r.bottom() + 1.0 {
            break;
        }
        let ir = Rect::from_min_size(pos2(r.left(), y), vec2(icon, icon));
        match a.icon.as_deref().and_then(|i| images.get(std::path::Path::new(i), ImgSize::Icon)) {
            Some(t) => {
                paint_image(&p, ir, &t, Fit::Cover, Color32::WHITE, look.h(0.008), 0.0);
            }
            None => {
                p.rect_filled(ir, cr(look.h(0.008)), fade(look.text, 0.12));
            }
        }
        let tx = ir.right() + look.h(0.014);
        let tw = (r.right() - tx).max(10.0);
        let name = truncated(&p, ach::shown_name(a), look.medium(0.019), look.text, tw);
        let when = truncated(&p, &ach::when_line(a), look.font(0.016), look.dim, tw);
        let gap = (icon - name.size().y - when.size().y).max(0.0) / 2.0;
        paint_galley(&p, pos2(tx, y + gap), &name, look.text, SHADOW, 0.0);
        paint_galley(&p, pos2(tx, y + gap + name.size().y), &when, look.dim, SHADOW, 0.0);
        y += icon + look.h(0.01);
    }
}

/// One line cut to `w` with "…".
fn truncated(p: &egui::Painter, text: &str, font: egui::FontId, c: Color32, w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, c);
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    p.layout_job(job)
}

/// The cover on the right, with a star badge on favourites.
fn paint_cover(page: &Page, images: &mut ImageCache, game: &Game) {
    let Page { p, look, screen, sw, sh } = *page;
    let Some(t) = tex(images, &game.art.cover, ImgSize::Medium) else {
        return;
    };
    let h = 0.5 * sh;
    let w = h * t.aspect_ratio().clamp(0.4, 1.2);
    let r = Rect::from_min_size(pos2(screen.right() - 0.07 * sw - w, screen.top() + 0.37 * sh), vec2(w, h));
    drop_shadow(p, r, look.h(0.014), 0.9);
    paint_image(p, r, &t, Fit::Cover, Color32::WHITE, look.h(0.014), 0.0);
    if game.favorite {
        let b =
            Rect::from_min_size(r.right_top() + vec2(-look.h(0.06), look.h(0.015)), vec2(look.h(0.045), look.h(0.045)));
        p.rect_filled(b, cr(b.height() / 2.0), Color32::from_black_alpha(150));
        star(p, b.center(), b.height() * 0.32, look.favorite);
    }
}

fn paint_help_bar(page: &Page, hints: &[Hint]) {
    let Page { p, look, screen, sw, sh } = *page;
    let hb = Rect::from_min_max(
        pos2(screen.left() + 0.04 * sw, screen.bottom() - 0.075 * sh),
        pos2(screen.right() - 0.045 * sw, screen.bottom() - 0.015 * sh),
    );
    look.paint_screen_hints(p, hb, hints, fade(look.text, 0.85));
}

fn protondb_text_on(fill: Color32) -> Color32 {
    let luma = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
    if luma > 140.0 { Color32::from_rgb(18, 20, 24) } else { Color32::WHITE }
}

/// A framed picture (mountains and a sun), `w` wide, centred at `c`: "view artwork".
fn picture_glyph(p: &egui::Painter, c: egui::Pos2, w: f32, col: Color32) {
    let h = w * 0.78;
    let r = Rect::from_center_size(c, vec2(w, h));
    let sw = (w * 0.09).max(1.0);
    p.rect_stroke(r, cr(w * 0.12), Stroke::new(sw, col), StrokeKind::Inside);
    let inner = r.shrink(sw * 1.6);
    p.circle_filled(pos2(inner.right() - inner.width() * 0.22, inner.top() + inner.height() * 0.28), w * 0.075, col);
    let at = |x: f32, y: f32| pos2(inner.left() + inner.width() * x, inner.top() + inner.height() * y);
    for tri in [[at(0.0, 1.0), at(0.36, 0.36), at(0.8, 1.0)], [at(0.45, 1.0), at(0.72, 0.52), at(1.0, 1.0)]] {
        p.add(egui::Shape::convex_polygon(tri.to_vec(), col, Stroke::NONE));
    }
}

/// An arrow out to the top right (opens another app), centred on `c`, `w` wide.
fn open_glyph(p: &egui::Painter, c: egui::Pos2, w: f32, col: Color32) {
    let st = Stroke::new((w * 0.16).max(1.5), col);
    let (a, b) = (pos2(c.x - w * 0.4, c.y + w * 0.4), pos2(c.x + w * 0.45, c.y - w * 0.45));
    p.line_segment([a, b], st);
    p.line_segment([b, pos2(b.x - w * 0.55, b.y)], st);
    p.line_segment([b, pos2(b.x, b.y + w * 0.55)], st);
}

/// Under the buttons of a game no longer in the account's Steam library: why its button opens
/// Steam instead of installing.
fn paint_steam_library_note(page: &Page, buttons_bottom: f32) {
    let Page { p, look, screen, sw, .. } = *page;
    // One line between the buttons and the stats: the short form of the desktop page's note.
    let text =
        "Not in your Steam library. It may be from a subscription that ended, such as EA Play, or a shared library.";
    let g = layout_line(p, text, look.medium(0.019), look.dim, 0.6 * sw);
    paint_galley(p, pos2(screen.left() + 0.06 * sw, buttons_bottom + look.h(0.013)), &g, look.dim, SHADOW, 0.0);
}

/// A download arrow over a tray, centred on `c`, `w` wide.
fn download_glyph(p: &egui::Painter, c: egui::Pos2, w: f32, col: Color32) {
    let st = Stroke::new((w * 0.16).max(1.5), col);
    let top = c.y - w * 0.55;
    let tip = c.y + w * 0.18;
    p.line_segment([pos2(c.x, top), pos2(c.x, tip)], st);
    let a = w * 0.34;
    p.line_segment([pos2(c.x - a, tip - a), pos2(c.x, tip)], st);
    p.line_segment([pos2(c.x + a, tip - a), pos2(c.x, tip)], st);
    let base = c.y + w * 0.52;
    p.line_segment([pos2(c.x - w * 0.5, base), pos2(c.x + w * 0.5, base)], st);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protondb_chip_colours_are_readable() {
        for t in crate::metadata::protondb::TIERS {
            let (fill, text) = protondb_colors(t).unwrap();
            assert_eq!(text, protondb_text_on(fill));
        }
        // Gold / platinum chips use dark text, borked (red) white.
        assert_ne!(protondb_colors("gold").unwrap().1, Color32::WHITE);
        assert_eq!(protondb_colors("borked").unwrap().1, Color32::WHITE);
        assert!(protondb_colors("shiny").is_none());
    }
}
