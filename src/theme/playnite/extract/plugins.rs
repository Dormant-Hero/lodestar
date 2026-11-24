//! What the theme takes from outside: the add-ons its views host, its options file, its
//! videos and sounds.

use super::*;

impl<'a, 'i> X<'a, 'i> {
    /// Add-ons the theme refers to: `{PluginSettings Plugin=X …}` / PluginStatus /
    /// PluginConverter, and add-on controls (`<ContentControl x:Name="Addon_Control" />`, the
    /// Playnite convention: the add-on's name, an underscore, the control's).
    pub(super) fn plugins(&self) -> Plugins {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut p = Plugins::default();
        let mut controls: Vec<&str> = vec![];
        for (_, el) in self.t.theme_elements() {
            for a in el.attributes() {
                let v = a.value();
                if !v.contains("Plugin") {
                    continue;
                }
                let mut rest = v;
                while let Some(i) = rest.find("Plugin=") {
                    let after = &rest[i + 7..];
                    let name: String = after
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .collect();
                    // "Name_<guid>" ids: the name.
                    let name = match name.rsplit_once('_') {
                        Some((pre, guid))
                            if guid.len() == 36 && guid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') =>
                        {
                            pre.to_string()
                        }
                        _ => name,
                    };
                    if !name.is_empty() {
                        *counts.entry(name).or_default() += 1;
                        p.uses += 1;
                    }
                    rest = after;
                }
            }
            if let Some(n) = name_of(el)
                && matches!(tag(el), "ContentControl" | "ContentPresenter")
                && n.contains('_')
                && !n.starts_with("PART_")
                && n.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                controls.push(n);
            }
        }
        let referenced: Vec<String> = counts.keys().cloned().collect();
        for n in controls {
            // The longest underscore prefix some Plugin= reference starts with, else the part
            // before the first underscore.
            let cuts: Vec<usize> = n.match_indices('_').map(|(i, _)| i).collect();
            let plugin = cuts
                .iter()
                .rev()
                .map(|&i| &n[..i])
                .find(|pre| referenced.iter().any(|r| r.starts_with(pre)))
                .unwrap_or(&n[..cuts[0]]);
            if plugin.len() <= 3 {
                continue;
            }
            let part = n[plugin.len() + 1..].to_ascii_lowercase();
            *counts.entry(plugin.to_string()).or_default() += 1;
            if part.contains("logo") {
                p.logos = true;
            }
            if plugin == "ThemeExtras" && part.contains("banner") {
                p.banners = true;
            }
            if plugin == "ThemeExtras"
                && (part.contains("rating") || part.contains("favorite") || part.contains("completion"))
            {
                p.ratings = true;
            }
        }
        if counts.contains_key("ExtraMetadataLoader") {
            p.logos = true;
        }
        let mut names: Vec<(String, usize)> = counts.into_iter().collect();
        names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        p.names = names;
        p
    }

    /// `thememodifier.yaml`: `Constants:` then `- "Section"` headings and
    /// `- Key: 'Label'` / `- Key(min,max): 'Label'` entries.
    pub(super) fn options(&self, yaml: &str) -> Vec<ThemeOption> {
        let mut out = vec![];
        let mut section = String::new();
        let mut in_constants = false;
        for line in yaml.lines() {
            let l = line.trim_end();
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                continue;
            }
            if !l.starts_with([' ', '\t', '-']) {
                in_constants = l.trim().trim_start_matches('\u{feff}').starts_with("Constants");
                continue;
            }
            if !in_constants {
                continue;
            }
            let Some(item) = l.trim().strip_prefix('-') else { continue };
            let item = item.trim();
            let unq = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            let Some((head, label)) = item.split_once(':').filter(|(h, _)| !h.trim().starts_with(['"', '\''])) else {
                section = unq(item);
                continue;
            };
            let head = head.trim();
            let (key, range) = match head.split_once('(') {
                Some((k, r)) => {
                    let r = r.trim_end_matches(')');
                    let nums: Vec<f64> = r.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    (k.trim(), (nums.len() == 2).then(|| (nums[0], nums[1])))
                }
                None => (head, None),
            };
            if key.is_empty() || out.len() >= 200 {
                continue;
            }
            let val = self.res_val(key);
            let kind = match range {
                Some((min, max)) => OptionKind::Range {
                    min,
                    max,
                    default: val.as_ref().and_then(|v| v.num().or_else(|| v.max_side().map(f64::from))),
                },
                None => OptionKind::Switch {
                    default: val.as_ref().and_then(|v| {
                        v.boolean().or_else(|| match v.text()? {
                            "Visible" => Some(true),
                            "Collapsed" | "Hidden" => Some(false),
                            _ => None,
                        })
                    }),
                },
            };
            out.push(ThemeOption { key: key.to_string(), label: unq(label), section: section.clone(), kind });
        }
        out
    }

    pub(super) fn media(&self) -> MediaLook {
        let mut m = MediaLook::default();
        let size_of = |f: &str| self.t.files.iter().find(|(x, _)| x == f).map(|(_, s)| *s).unwrap_or(0);
        // Playnite's own audio folder.
        let audio = |stem_want: &str| -> Option<String> {
            self.t
                .files
                .iter()
                .filter(|(f, size)| {
                    let l = f.to_ascii_lowercase();
                    *size > 0
                        && l.starts_with("audio/")
                        && l.matches('/').count() == 1
                        && stem(&l) == stem_want
                        && SOUND_EXTS.contains(&ext(&l).as_str())
                })
                .map(|(f, _)| f.clone())
                .next()
        };
        if let Some(f) = audio("background") {
            m.music.push(f);
        }
        for (want, role) in [("navigation", SoundRole::Navigation), ("activation", SoundRole::Activation)] {
            if let Some(f) = audio(want) {
                m.sounds.push(ThemeSound { role, bytes: size_of(&f), file: f });
            }
        }
        // MediaElements in the theme's XAML.
        let mut seen: BTreeSet<String> = m.music.iter().chain(m.sounds.iter().map(|s| &s.file)).cloned().collect();
        for (doc, el) in self.t.theme_elements() {
            if tag(el) != "MediaElement" {
                continue;
            }
            let mut sources: Vec<String> = vec![];
            if let Some(Value::ThemeFile(f)) = self.t.local(el, "Source") {
                sources.push(f);
            }
            // Sources set by the element's style / triggers.
            for set in el.descendants().filter(|n| n.is_element() && tag(*n) == "Setter") {
                if attr(set, "Property").is_some_and(|p| p.trim() == "Source")
                    && let Some(Value::ThemeFile(f)) = self.t.setter_value(set, "Source")
                {
                    sources.push(f);
                }
            }
            let view = stem(doc.rel).to_string();
            for src in sources {
                let Some(file) = theme_file(&self.t.files, &src) else { continue };
                if !seen.insert(file.clone()) {
                    continue;
                }
                let e = ext(&file);
                let bytes = size_of(&file);
                if bytes == 0 {
                    continue;
                }
                if VIDEO_EXTS.contains(&e.as_str()) {
                    let repeat =
                        attr(el, "MediaElementBehaviors.Repeat").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
                    let lname = file.to_ascii_lowercase();
                    let sized = self.num(el, "Width").is_some() || self.num(el, "Height").is_some();
                    let role = if lname.contains("intro") || lname.contains("startup") || lname.contains("start up") {
                        VideoRole::Intro
                    } else if !sized && (repeat || matches!(view.as_str(), "Main" | "GameDetails")) {
                        VideoRole::Background
                    } else {
                        VideoRole::Other
                    };
                    m.videos.push(ThemeVideo {
                        file,
                        bytes,
                        role,
                        view: view.clone(),
                        repeat,
                        muted: attr(el, "IsMuted").is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
                        stretch: self.text(el, "Stretch").and_then(|s| Stretch::parse(&s)),
                        opacity: self.num(el, "Opacity").filter(|o| *o < 0.999),
                    });
                } else if SOUND_EXTS.contains(&e.as_str()) {
                    m.sounds.push(ThemeSound { role: SoundRole::Other(stem(&file).to_string()), file, bytes });
                }
            }
        }
        let rank = |v: &ThemeVideo| match (v.role, v.view.as_str()) {
            (VideoRole::Background, "Main") => 0,
            (VideoRole::Background, _) => 1,
            (VideoRole::Intro, _) => 3,
            _ => 2,
        };
        m.videos.sort_by_key(rank);
        let used: BTreeSet<&str> = m.videos.iter().map(|v| v.file.as_str()).collect();
        m.unused_videos = self
            .t
            .files
            .iter()
            .filter(|(f, s)| *s > 0 && VIDEO_EXTS.contains(&ext(f).as_str()) && !used.contains(f.as_str()))
            .map(|(f, _)| f.clone())
            .collect();
        m.bytes = m.videos.iter().map(|v| v.bytes).sum::<u64>()
            + m.sounds.iter().map(|s| s.bytes).sum::<u64>()
            + m.music.iter().map(|f| size_of(f)).sum::<u64>();
        m
    }
}
