//! Fonts by role: the theme's text styles' families, weights and sizes.

use super::*;

impl<'a, 'i> X<'a, 'i> {
    /// A FontFamily as written (attribute / setter text) → role.
    pub(super) fn font_role(&self, at: N<'a, 'i>, raw: &str, weight: Option<String>) -> Option<FontRole> {
        let mut role = match markup(raw) {
            Markup::Lit(v) => {
                FontRole { value: Some(v.to_string()), family: pfonts::family_of_value(v), ..Default::default() }
            }
            Markup::Resource(k) => match self.t.resource_from(at, &k).and_then(|n| self.t.eval_node(n, 0)) {
                Some(Value::Text(v)) => FontRole {
                    value: Some(v.clone()),
                    family: pfonts::family_of_value(&v),
                    key: Some(k),
                    ..Default::default()
                },
                _ => {
                    let (family, style) = pfonts::family_of_key(&k)?;
                    FontRole {
                        key: Some(k),
                        family: Some(family),
                        weight: pfonts::weight_of(&style),
                        ..Default::default()
                    }
                }
            },
            _ => return None,
        };
        if role.weight == 0 {
            role.weight = role
                .key
                .as_deref()
                .and_then(pfonts::family_of_key)
                .map(|(_, s)| pfonts::weight_of(&s))
                .or_else(|| role.value.as_deref().map(|v| pfonts::weight_of(v.rsplit('#').next().unwrap_or(v))))
                .unwrap_or(400);
        }
        if let Some(w) = weight {
            role.weight = pfonts::weight_of(&w);
        }
        Some(role)
    }

    /// The raw text of a property on an element or from its style chain (not resolved).
    pub(super) fn raw(&self, el: N<'a, 'i>, prop: &str) -> Option<(N<'a, 'i>, &'a str)> {
        if let Some(v) = attr(el, prop) {
            return Some((el, v));
        }
        let mut s = self.t.style_of(el).or_else(|| (tag(el) == "Style").then_some(el));
        for _ in 0..16 {
            let st = s?;
            if let Some(set) =
                content(st).filter(|c| tag(*c) == "Setter" && attr(*c, "TargetName").is_none()).find(|c| {
                    attr(*c, "Property").is_some_and(|p| p.trim() == prop || p.trim().ends_with(&format!(".{prop}")))
                })
            {
                return attr(set, "Value").map(|v| (set, v));
            }
            s = self.t.based_on(st);
        }
        None
    }

    pub(super) fn font_role_of(&self, el: N<'a, 'i>) -> Option<FontRole> {
        let (at, raw) = self.raw(el, "FontFamily")?;
        let weight = self.raw(el, "FontWeight").map(|(_, w)| w.to_string()).filter(|w| !w.starts_with('{'));
        self.font_role(at, raw, weight)
    }

    /// The body font: what the theme itself sets first (FontFamily resource, the base text
    /// styles), then Playnite's defaults.
    pub(super) fn body_font(&self) -> Option<FontRole> {
        let styles: &[&str] = match self.t.mode {
            Mode::Desktop => &["BaseTextBlockStyle", "TextBlockBaseStyle"],
            Mode::Fullscreen => &["TextBlockBaseStyle", "BaseTextBlockStyle"],
        };
        let from_style = |s: &str| {
            let st = self.t.resource(s).filter(|n| tag(*n) == "Style")?;
            self.font_role_of(st)
        };
        let from_res = || {
            let n = self.t.resource("FontFamily")?;
            match self.t.eval_node(n, 0)? {
                Value::Text(v) => Some(FontRole {
                    family: pfonts::family_of_value(&v),
                    weight: 400,
                    value: Some(v),
                    key: Some("FontFamily".into()),
                }),
                _ => None,
            }
        };
        for theme_pass in [true, false] {
            if self.t.theme_defines("FontFamily") == theme_pass
                && let Some(r) = from_res()
            {
                return Some(r);
            }
            for s in styles {
                if self.t.theme_defines(s) == theme_pass
                    && let Some(r) = from_style(s)
                {
                    return Some(r);
                }
            }
        }
        None
    }
}
