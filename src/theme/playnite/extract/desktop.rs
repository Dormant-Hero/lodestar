//! The desktop look: the main window's background and picture, the game art behind it (its
//! masks, place and the layers over it), the panels' fills, the grid tiles, the game list
//! column, the facts box, the play button, the title font.

use super::*;

/// Playnite's default darkening of the window's game art (BackgroundImageDarkAmount 0.7).
const PLAYNITE_ART_DARKEN: u8 = 179;
/// A list painted at least this opaque hides what is behind it.
const OPAQUE_ALPHA: u8 = 230;
/// A picture drawn fainter than this is grain over the window, not a picture.
const MIN_PICTURE_OPACITY: f32 = 0.1;
/// The game page's fact rows (any one of them shows where the facts sit).
const FACT_PARTS: [&str; 8] = [
    "PART_ElemPlayTime",
    "PART_ElemLastPlayed",
    "PART_ElemCompletionStatus",
    "PART_ElemPlatform",
    "PART_ElemGenres",
    "PART_ElemDevelopers",
    "PART_ElemReleaseDate",
    "PART_ElemLibrary",
];

/// A template's own ContentPresenter (not one inside a nested control's template).
fn content_presenter<'a, 'i>(scope: N<'a, 'i>) -> Option<N<'a, 'i>> {
    scope.descendants().find(|n| {
        n.is_element()
            && tag(*n) == "ContentPresenter"
            && n.ancestors().find(|a| matches!(tag(*a), "ControlTemplate" | "DataTemplate")) == Some(scope)
    })
}

/// Nothing behind it shows through (a solid colour or a gradient without any transparency).
fn is_opaque(b: &LookBrush) -> bool {
    match b {
        LookBrush::Solid { color } => color.0[3] == 255,
        LookBrush::Linear { stops, .. } | LookBrush::Radial { stops, .. } => {
            !stops.is_empty() && stops.iter().all(|(c, _)| c.0[3] == 255)
        }
        LookBrush::Image { .. } => false,
    }
}

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
        (d.grid.background, d.caption_fill) = self.grid_item_fills();
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
        // Lodestar's side panel is Playnite's explorer (sources, platforms …); the theme's
        // narrow Sidebar strip of icons, where it paints one, is the closest otherwise.
        d.sidebar = view_bg("ExplorerPanel").or_else(|| view_bg("Sidebar"));
        d.details_background = view_bg("DetailsViewGameOverview").or_else(|| view_bg("GridViewGameOverview"));
        d.game_list = view_bg("LibraryDetailsView");
        d.list_fill = self.list_fill();
        d.window_art = self.window_art(d.game_list.as_ref());
        if let Some(art) = &mut d.window_art
            && let Some(over) = self.picture_over_art()
        {
            art.picture_over_art = true;
            d.background_picture = Some(over);
        }
        d.facts_box = self.facts_box();
        d.rows = self.list_rows();
        d.play_button = self.play_button();
        if let Some(tpl) = self.t.template_of("DetailsViewGameOverview") {
            d.details_picture = self.t.named(tpl, "PART_ImageBackground").map(|p| self.picture(p, None));
            d.title_size = self.t.named(tpl, "PART_TextDisplayName").and_then(|t| self.num(t, "FontSize"));
        }
        d
    }

    /// What the grid item paints at rest: a tile behind the cover and the name under it (an
    /// element around both, or around the item style's own ContentPresenter), and a strip
    /// behind that name alone. A fill around the cover only stays under the picture. Fills a
    /// trigger clears at rest don't count (Playnite's own item clears its
    /// GridItemBackgroundBrush unless the user turns on "show grid item background"), nor
    /// do frames (BorderBrush) or what a trigger sets only sometimes.
    fn grid_item_fills(&self) -> (Option<LookBrush>, Option<LookBrush>) {
        let style = self.t.resource("GridViewItemStyle").filter(|n| tag(*n) == "Style");
        let container = style.and_then(|s| self.t.template(s));
        let scopes: Vec<N> = [container, self.t.template_of("GridViewItemTemplate")].into_iter().flatten().collect();
        // The item style's ContentPresenter is where the item template (cover and name) goes.
        let presenter = container.and_then(content_presenter);
        let cover = scopes.iter().find_map(|s| self.t.named(*s, "PART_ImageCover"));
        // Names over the cover (an empty cover's title, a hover strip) aren't the name under it.
        let names_under: Vec<N> = scopes
            .iter()
            .flat_map(|s| s.descendants())
            .filter(|n| n.is_element() && tag(*n) == "TextBlock")
            .filter(|n| self.t.local(*n, "Text").as_ref().is_some_and(binds_name))
            .filter(|n| cover.is_none_or(|c| !self.same_cell(c, *n)))
            .collect();
        let holds = |e: N<'a, 'i>, inner: N<'a, 'i>| inner == e || inner.ancestors().any(|a| a == e);
        let (mut tile, mut strip) = (None, None);
        for scope in &scopes {
            for e in scope.descendants().filter(|n| n.is_element() && !tag(*n).contains('.')) {
                let Some(fill) = self.item_fill_at_rest(e, *scope, style) else { continue };
                let has_cover = cover.is_some_and(|c| holds(e, c));
                let has_name = names_under.iter().any(|n| holds(e, *n));
                if presenter.is_some_and(|p| holds(e, p)) || (has_cover && has_name) {
                    tile = tile.or(Some(fill));
                } else if has_name && !has_cover {
                    strip = strip.or(Some(fill));
                }
            }
        }
        (tile, strip)
    }

    /// The fill an element of an item template shows at rest: its Background (a Rectangle's
    /// Fill; `{TemplateBinding Background}`: the item style's), unless it is hidden or a
    /// trigger that holds at rest sets another one (Transparent / null clears it).
    fn item_fill_at_rest(&self, e: N<'a, 'i>, scope: N<'a, 'i>, style: Option<N<'a, 'i>>) -> Option<LookBrush> {
        // What a trigger sets applies only sometimes; a nested control's template or style
        // isn't the item's.
        let nested = |a: N<'a, 'i>| {
            tag(a).ends_with("Trigger") || matches!(tag(a), "Style" | "ControlTemplate" | "DataTemplate")
        };
        if e.ancestors().take_while(|a| *a != scope).any(nested) {
            return None;
        }
        let prop = fill_prop(e);
        let bound = attr(e, prop).is_some_and(|v| v.contains("TemplateBinding"));
        let own = if bound {
            style.and_then(|s| self.t.setter(s, prop)).and_then(|v| v.brush())
        } else {
            self.t.get(e, prop).and_then(|v| v.brush())
        };
        let own = own.filter(|b| !b.is_clear())?;
        if self.hidden(e) || self.hidden_up_to(e, scope).is_some() {
            return None;
        }
        let resting = self.t.triggers(scope).into_iter().find_map(|trig| {
            if self.t.static_trigger(e, trig) != Some(true) {
                return None;
            }
            content(trig)
                .filter(|set| tag(*set) == "Setter" && attr(*set, "Property").map(str::trim) == Some(prop))
                .find(|set| attr(*set, "TargetName").is_some_and(|t| Some(t.trim()) == name_of(e)))
                .map(|set| self.t.setter_value(set, prop).and_then(|v| v.brush()))
        });
        match resting {
            Some(set) => set.filter(|b| !b.is_clear()),
            None => Some(own),
        }
    }

    /// The selected game's art that the Library view draws behind everything
    /// (PART_ImageBackground): Playnite shows it in the details view unless the user turns it
    /// off. Its darkening is the user's setting (Playnite sets the ImageDarkeningBrush from
    /// code, over whatever the theme wrote); the theme's masks, its place in the view and the
    /// layers it lays right over it come along. None when the theme hides it, or covers it
    /// with an opaque layer or game list.
    fn window_art(&self, game_list: Option<&LookBrush>) -> Option<WindowArt> {
        let tpl = self.t.template_of("Library")?;
        let root = self.t.template_root(tpl)?;
        let p = self.t.named(tpl, "PART_ImageBackground")?;
        if self.hidden(p) || self.hidden_up_to(p, root).is_some() {
            return None;
        }
        if game_list.and_then(LookBrush::average).is_some_and(|c| c.0[3] >= OPAQUE_ALPHA) {
            return None;
        }
        let layers = self.art_layers(p, root);
        if layers.iter().any(is_opaque) {
            return None;
        }
        let mut art = self.picture(p, None);
        art.darken = Some(Color([0, 0, 0, PLAYNITE_ART_DARKEN]));
        Some(WindowArt {
            picture: art,
            masks: self.art_masks(p, root),
            layers,
            below_top_bar: self.below_top_panel(p, root),
            picture_over_art: false,
        })
    }

    /// A theme picture the Library view draws after the game art (later in the document),
    /// so over it: a vignette, say. Small tiled textures (noise) aren't pictures to stretch.
    fn picture_over_art(&self) -> Option<LookBrush> {
        let tpl = self.t.template_of("Library").filter(|t| self.t.layer_of(*t) == Layer::Theme)?;
        let p = self.t.named(tpl, "PART_ImageBackground")?;
        let root = self.t.template_root(tpl)?;
        tpl.descendants()
            .skip_while(|n| *n != p)
            .filter(|n| n.is_element() && !self.hidden(*n) && self.hidden_up_to(*n, root).is_none())
            .filter_map(|el| self.theme_picture(el))
            .find(|pic| self.is_picture(pic) && pic.alpha_at(0.5, 0.5) >= MIN_PICTURE_OPACITY)
    }

    /// Gradient opacity masks on the art and on the containers it sits in (up to the view).
    fn art_masks(&self, p: N<'a, 'i>, root: N<'a, 'i>) -> Vec<LookBrush> {
        let gradient = |b: &LookBrush| matches!(b, LookBrush::Linear { .. } | LookBrush::Radial { .. });
        let mask = |el: N<'a, 'i>, prop: &str| self.t.get(el, prop).and_then(|v| v.brush()).filter(gradient);
        let own = mask(p, "ImageOpacityMask").into_iter().chain(mask(p, "OpacityMask"));
        let around = p
            .ancestors()
            .filter(|a| a.is_element() && !tag(*a).contains('.'))
            .take_while(|a| *a != root)
            .filter_map(|a| mask(a, "OpacityMask"));
        own.chain(around).collect()
    }

    /// Plain layers drawn right after the art over the same area (an empty Border, Grid or
    /// Rectangle with a fill, or a Grid holding only such layers), up to the first element
    /// with other content (the views): its own siblings and, where it sits alone in a
    /// wrapper, the wrapper's.
    fn art_layers(&self, p: N<'a, 'i>, root: N<'a, 'i>) -> Vec<LookBrush> {
        let mut layers = vec![];
        let mut cur = p;
        loop {
            // (`next_siblings` starts with the node itself.)
            for s in cur.next_siblings().skip(1).filter(|n| n.is_element() && !tag(*n).contains('.')) {
                if self.hidden(s) {
                    continue;
                }
                if !self.is_layer(s) {
                    return layers;
                }
                if self.fills_parent(s) && self.same_cell(p, s) {
                    self.layer_brushes(s, 1.0, &mut layers);
                }
            }
            match cur.parent() {
                Some(up) if up != root && content(up).count() == 1 => cur = up,
                _ => return layers,
            }
        }
    }

    /// A plain layer: an empty Border / Grid / Rectangle, or a Grid of such layers.
    fn is_layer(&self, el: N<'a, 'i>) -> bool {
        match tag(el) {
            "Border" | "Rectangle" => content(el).next().is_none(),
            "Grid" => content(el).all(|c| self.is_layer(c)),
            _ => false,
        }
    }

    /// The fills a layer draws (a group's layers in order), at the opacity of the elements
    /// they sit in. Picture brushes (grain textures) aren't colours to draw over the art, and
    /// a masked layer (a halftone of dots, say) isn't an even wash.
    fn layer_brushes(&self, el: N<'a, 'i>, opacity: f32, out: &mut Vec<LookBrush>) {
        if self.hidden(el) || self.t.get(el, "OpacityMask").is_some() {
            return;
        }
        let opacity = opacity * self.num(el, "Opacity").unwrap_or(1.0).clamp(0.0, 1.0);
        let prop = fill_prop(el);
        if let Some(b) = self.brush(el, prop).filter(|b| !matches!(b, LookBrush::Image { .. })) {
            out.push(b.with_opacity(opacity));
        }
        for c in content(el) {
            self.layer_brushes(c, opacity, out);
        }
    }

    /// The art sits in a row of the view's Grid below the TopPanel's (Grid.Row of the view's
    /// children holding each).
    fn below_top_panel(&self, p: N<'a, 'i>, root: N<'a, 'i>) -> bool {
        let row = |el: N<'a, 'i>| {
            std::iter::once(el)
                .chain(el.ancestors())
                .find(|x| x.parent() == Some(root))
                .map(|c| grid_attr(c, "Grid.Row", 0))
        };
        let top = root.descendants().find(|n| n.is_element() && tag(*n) == "TopPanel");
        matches!((row(p), top.and_then(row)), (Some(art), Some(bar)) if art > bar)
    }

    /// The game list column's fill: the list's own Background where its template draws one
    /// (as set on it, by its style, by the ListBox style it inherits, else Playnite's stock
    /// ListBox: ControlBackgroundBrush), else that of the nearest panel around it inside the
    /// details view (not one that also holds the game's overview: that is the view's own fill).
    fn list_fill(&self) -> Option<LookBrush> {
        let tpl = self.t.template_of("LibraryDetailsView")?;
        let root = self.t.template_root(tpl)?;
        let list = self.t.named(tpl, "PART_ListGames")?;
        let styles = [self.t.style_of(list), self.t.implicit_style("ListBox")];
        let styled = |prop: &str| styles.iter().flatten().find_map(|s| self.t.setter(*s, prop));
        let own = self
            .t
            .local(list, "Background")
            .or_else(|| styled("Background"))
            .and_then(|v| v.brush())
            .or_else(|| self.res_brush("ControlBackgroundBrush"));
        // A template of the theme's that never draws the Background leaves the list clear.
        let drawn = styled("Template").and_then(|v| v.node()).is_none_or(|t| {
            t.descendants().any(|n| n.attributes().any(|a| a.value().contains("TemplateBinding Background")))
        });
        if let Some(b) = own.filter(|b| drawn && !b.is_clear()) {
            return Some(b);
        }
        let overview = self.t.named(tpl, "PART_ControlGameView");
        list.ancestors()
            .filter(|a| a.is_element() && !tag(*a).contains('.'))
            .take_while(|a| *a != root)
            .take_while(|a| overview.is_none_or(|o| !o.ancestors().any(|x| x == *a)))
            .find_map(|a| self.brush(a, "Background"))
    }

    /// The box the game page draws around its facts: the nearest element around them with a
    /// fill of its own that doesn't also hold the description (that is the page's panel).
    fn facts_box(&self) -> Option<FactsBox> {
        let tpl = self.t.template_of("DetailsViewGameOverview")?;
        let root = self.t.template_root(tpl)?;
        let fact = tpl.descendants().find(|n| name_of(*n).is_some_and(|name| FACT_PARTS.contains(&name)))?;
        let description = self.t.named(tpl, "PART_HtmlDescription");
        let around = fact.ancestors().filter(|a| a.is_element() && !tag(*a).contains('.'));
        for a in around.take_while(|a| *a != root) {
            if description.is_some_and(|d| d.ancestors().any(|x| x == a)) {
                return None;
            }
            if let Some(fill) = self.brush(a, "Background") {
                let width = self.t.get(a, "BorderThickness").and_then(|v| v.max_side()).filter(|w| *w > 0.0);
                return Some(FactsBox {
                    fill,
                    border: self.brush(a, "BorderBrush").zip(width),
                    radius: self.t.get(a, "CornerRadius").and_then(|v| v.max_side()),
                });
            }
        }
        None
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

    /// The style of the game views' Play button (PART_ButtonPlayAction) as WPF applies it: the
    /// one the button names (a theme may give its views their own, e.g. `PlayButtonViews`), or
    /// the implicit Button style when it names none; `PlayButton` when no view has the button.
    fn play_button_style(&self) -> Option<N<'a, 'i>> {
        let button = ["DetailsViewGameOverview", "GridViewGameOverview"]
            .iter()
            .find_map(|view| self.t.named(self.t.template_of(view)?, "PART_ButtonPlayAction"));
        match button {
            Some(b) => self.t.style_of(b),
            None => self.t.resource("PlayButton"),
        }
        .filter(|n| tag(*n) == "Style" && self.t.layer_of(*n) == Layer::Theme)
    }

    /// The colour the theme's Play button is filled with at rest (its own style): the style's
    /// Background where its template draws it, else the brush of the template's frame (and
    /// what a resting `IsMouseOver = False` trigger sets on it).
    pub(super) fn play_button(&self) -> Option<LookBrush> {
        let style = self.play_button_style()?;
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
        let prop = fill_prop(frame);
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
    /// main window or library).
    fn window_picture(&self) -> Option<LookBrush> {
        ["MainWindow", "Library"]
            .into_iter()
            .filter_map(|ty| self.t.template_of(ty).filter(|t| self.t.layer_of(*t) == Layer::Theme))
            .find_map(|tpl| tpl.descendants().filter(|n| n.is_element()).find_map(|el| self.theme_picture(el)))
    }

    /// A theme file big enough to be a picture rather than a tile.
    fn is_picture(&self, b: &LookBrush) -> bool {
        let LookBrush::Image { file: Some(f), .. } = b else { return false };
        self.t.files.iter().any(|(p, size)| p == f && *size >= super::super::TEXTURE_BYTES)
    }

    /// The theme file an element draws (an Image's Source, an ImageBrush Background).
    fn theme_picture(&self, el: N<'a, 'i>) -> Option<LookBrush> {
        let src = match tag(el) {
            "Image" => self.t.get(el, "Source"),
            _ => self.t.get(el, "Background").filter(|v| matches!(v, Value::Brush(LookBrush::Image { .. }))),
        };
        match src? {
            Value::ThemeFile(f) => Some(LookBrush::Image {
                file: Some(theme_file(&self.t.files, &f)?),
                source: format!("{{ThemeFile '{f}'}}"),
                stretch: self.text(el, "Stretch").and_then(|s| Stretch::parse(&s)).unwrap_or_default(),
                opacity: self.num(el, "Opacity").unwrap_or(1.0),
            }),
            Value::Brush(b @ LookBrush::Image { file: Some(_), .. }) => {
                Some(b.with_opacity(self.num(el, "Opacity").unwrap_or(1.0).clamp(0.0, 1.0)))
            }
            _ => None,
        }
    }

    pub(super) fn desktop_title_font(&self) -> Option<FontRole> {
        let tpl = self.t.template_of("DetailsViewGameOverview")?;
        let t = self.t.named(tpl, "PART_TextDisplayName")?;
        self.font_role_of(t)
    }
}
