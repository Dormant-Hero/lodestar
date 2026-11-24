//! The desktop look: the main window's background and picture, the play button, the title
//! font.

use super::*;

impl<'a, 'i> X<'a, 'i> {
    pub(super) fn desktop_look(&self) -> DesktopLook {
        let mut d = DesktopLook {
            control_radius: self.res_val("ControlCornerRadius").and_then(|v| v.max_side()),
            grid: self.item_look("GridViewItemStyle", "GridViewItemTemplate", &["IsSelected"]),
            panel_separator: self.res_brush("PanelSeparatorBrush").filter(|b| !b.is_clear()),
            ..Default::default()
        };
        d.cover_title = match &d.grid.caption {
            None => CoverTitle::None,
            Some(c) if c.shown == Shown::Hover => CoverTitle::Hover,
            Some(c) if c.overlay => CoverTitle::Overlay,
            Some(_) => CoverTitle::Under,
        };
        if d.grid.background.is_none() {
            d.grid.background = self.res_brush("GridItemBackgroundBrush").filter(|b| !b.is_clear());
        }
        // Window: MainWindowStyle's Background (WindowBackgourndBrush in the default).
        let win_style = self.t.resource("MainWindowStyle").filter(|n| tag(*n) == "Style");
        // Window: MainWindowStyle's Background, else the window type's own style (a theme may
        // set it there and leave MainWindowStyle without one), the theme's own before
        // Playnite's; else Playnite's window brush.
        let styles = [win_style, self.t.implicit_style("WindowBase")];
        let style_bg = |s: N<'a, 'i>| self.t.setter(s, "Background").and_then(|v| v.brush());
        let theme_first = styles
            .iter()
            .flatten()
            .filter(|s| self.t.layer_of(**s) == Layer::Theme)
            .chain(styles.iter().flatten())
            .find_map(|s| style_bg(*s));
        d.window_background = theme_first.or_else(|| self.res_brush("WindowBackgourndBrush")).filter(|b| !b.is_clear());
        if let Some(root) = win_style.and_then(|s| self.t.template(s)).and_then(|t| self.t.template_root(t)) {
            // The window's outer Border (the root, or just inside an AdornerDecorator).
            let border = std::iter::once(root).chain(content(root)).find(|n| tag(*n) == "Border");
            d.window_radius =
                border.and_then(|b| self.t.get(b, "CornerRadius")).and_then(|v| v.max_side()).filter(|r| *r > 0.0);
        }
        if matches!(d.window_background, Some(LookBrush::Image { .. })) {
            d.background_picture = d.window_background.clone();
        } else {
            d.background_picture = self.window_picture();
        }
        // A view's fill: its root's Background, else that of a first child that covers the
        // root (not a thin stripe or a badge in a corner).
        let view_bg = |ty: &str| -> Option<LookBrush> {
            let tpl = self.t.template_of(ty)?;
            let root = self.t.template_root(tpl)?;
            self.brush(root, "Background").or_else(|| {
                content(root).next().filter(|c| self.fills_parent(*c)).and_then(|c| self.brush(c, "Background"))
            })
        };
        d.top_panel = view_bg("TopPanel");
        d.sidebar = view_bg("Sidebar");
        d.details_background = view_bg("DetailsViewGameOverview");
        d.play_button = self.play_button();
        if let Some(tpl) = self.t.template_of("DetailsViewGameOverview") {
            d.details_picture = self.t.named(tpl, "PART_ImageBackground").map(|p| self.picture(p, None));
            d.title_size = self.t.named(tpl, "PART_TextDisplayName").and_then(|t| self.num(t, "FontSize"));
        }
        d
    }

    /// The element stretches over its parent: no size of its own, stretched both ways.
    pub(super) fn fills_parent(&self, el: N<'a, 'i>) -> bool {
        let stretched = |p: &str| self.text(el, p).is_none_or(|a| a.trim().eq_ignore_ascii_case("stretch"));
        self.num(el, "Width").is_none()
            && self.num(el, "Height").is_none()
            && self.num(el, "MaxWidth").is_none()
            && self.num(el, "MaxHeight").is_none()
            && stretched("HorizontalAlignment")
            && stretched("VerticalAlignment")
    }

    /// The colour the theme's Play button is filled with at rest (its own `PlayButton` style):
    /// the style's Background where its template draws it, else the brush of the template's
    /// frame (and what a resting `IsMouseOver = False` trigger sets on it).
    pub(super) fn play_button(&self) -> Option<LookBrush> {
        let style =
            self.t.resource("PlayButton").filter(|n| tag(*n) == "Style" && self.t.layer_of(*n) == Layer::Theme)?;
        let opaque = |b: &LookBrush| !b.is_clear() && b.average().is_some_and(|c| c.0[3] >= 128);
        let own = self.t.setter(style, "Background").and_then(|v| v.brush()).filter(opaque);
        let Some(tpl) = self.t.template(style) else { return own };
        let draws_own =
            tpl.descendants().any(|n| n.attributes().any(|a| a.value().contains("TemplateBinding Background")));
        if draws_own && own.is_some() {
            return own;
        }
        let Some(root) = self.t.template_root(tpl) else { return own };
        let frame = root
            .descendants()
            .filter(|n| n.is_element())
            .take(8)
            .find(|n| matches!(tag(*n), "Border" | "Rectangle") && self.fills_parent(*n));
        let Some(frame) = frame else { return own };
        let prop = if tag(frame) == "Rectangle" { "Fill" } else { "Background" };
        let resting = name_of(frame).and_then(|name| {
            self.t.triggers(tpl).into_iter().find_map(|tr| {
                let at_rest = attr(tr, "Property") == Some("IsMouseOver")
                    && attr(tr, "Value").is_some_and(|v| v.trim().eq_ignore_ascii_case("false"));
                if !at_rest {
                    return None;
                }
                content(tr)
                    .find(|set| {
                        tag(*set) == "Setter"
                            && attr(*set, "TargetName") == Some(name)
                            && attr(*set, "Property") == Some(prop)
                    })
                    .and_then(|set| self.t.setter_value(set, prop))
                    .and_then(|v| v.brush())
            })
        });
        resting.filter(opaque).or_else(|| self.t.get(frame, prop).and_then(|v| v.brush()).filter(opaque)).or(own)
    }

    /// A theme picture behind the window (an Image / ImageBrush naming a theme file in the
    /// main window, library or a background-ish resource).
    pub(super) fn window_picture(&self) -> Option<LookBrush> {
        for ty in ["MainWindow", "Library"] {
            let Some(tpl) = self.t.template_of(ty) else { continue };
            if self.t.layer_of(tpl) != Layer::Theme {
                continue;
            }
            for el in tpl.descendants().filter(|n| n.is_element()) {
                let src = match tag(el) {
                    "Image" => self.t.get(el, "Source"),
                    _ => self.t.get(el, "Background").filter(|v| matches!(v, Value::Brush(LookBrush::Image { .. }))),
                };
                match src {
                    Some(Value::ThemeFile(f)) => {
                        let file = theme_file(&self.t.files, &f)?;
                        return Some(LookBrush::Image {
                            file: Some(file),
                            source: format!("{{ThemeFile '{f}'}}"),
                            stretch: self.text(el, "Stretch").and_then(|s| Stretch::parse(&s)).unwrap_or_default(),
                            opacity: self.num(el, "Opacity").unwrap_or(1.0),
                        });
                    }
                    Some(Value::Brush(b @ LookBrush::Image { file: Some(_), .. })) => return Some(b),
                    _ => {}
                }
            }
        }
        None
    }

    pub(super) fn desktop_title_font(&self) -> Option<FontRole> {
        let tpl = self.t.template_of("DetailsViewGameOverview")?;
        let t = self.t.named(tpl, "PART_TextDisplayName")?;
        self.font_role_of(t)
    }
}
