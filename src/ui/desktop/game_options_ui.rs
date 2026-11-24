//! The game options editor: one list of plain switches and choices instead of
//! launch options to learn. Used twice:
//! * Settings → Games: the global defaults ("Auto" = Lodestar adds nothing);
//! * a game's Launch options dialog: per-game overrides ("Default (On)" = follow Settings), with
//!   what happened to each option in the current launch (applied, not possible here and why).
//!
//! Rows come from `game_options::OPTIONS` (labels, help, groups, parents); sub-settings show only
//! while their parent is on; rarely needed rows sit behind "Show advanced options".

use super::Pal;
use crate::game_options::{
    self as go, Caps, FlagSetting, GameOptions, GsFilter, GsScaler, InternalRes, Kind, Nvapi, OptionId, OptionState,
    Res, Tri, WindowMode,
};
use egui::{RichText, Ui, vec2};

/// What the editor needs besides the options it edits.
pub(super) struct EditorCtx<'a> {
    /// Detected capabilities; None until the first probe finished (nothing is greyed then).
    pub caps: Option<&'a Caps>,
    /// Per-game editor: the global defaults the "Default" choice follows. None = the global editor.
    pub global: Option<&'a GameOptions>,
    /// Per-game editor: what the current launch did with each option.
    pub report: &'a [(OptionId, Option<String>, OptionState)],
    /// egui id salt (two editors may be alive at once).
    pub salt: &'a str,
}

/// Draw the editor; true when `o` changed.
pub(super) fn options_editor(ui: &mut Ui, o: &mut GameOptions, cx: &EditorCtx, pal: &Pal) -> bool {
    let mut changed = false;
    let adv_id = egui::Id::NULL.with(("ls_go_adv", cx.salt));
    let mut show_adv: bool = ui.ctx().data_mut(|d| *d.get_persisted_mut_or(adv_id, false));
    ui.horizontal(|ui| {
        if ui.checkbox(&mut show_adv, "Show advanced options").changed() {
            ui.ctx().data_mut(|d| d.insert_persisted(adv_id, show_adv));
        }
        if cx.caps.is_none() {
            ui.label(RichText::new("Checking what's installed…").size(12.0).color(pal.dim));
        }
    });
    let eff = match cx.global {
        Some(g) => go::effective(g, o),
        None => go::effective(o, &GameOptions::default()),
    };
    for group in go::Group::ALL {
        let rows: Vec<&go::OptionMeta> = go::OPTIONS
            .iter()
            .filter(|m| m.group == group && (show_adv || !m.advanced))
            .filter(|m| row_visible(&eff, m))
            .collect();
        let layers = if group == go::Group::Advanced && show_adv { extra_layers(cx.caps) } else { vec![] };
        if rows.is_empty() && layers.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(RichText::new(group.label()).strong().size(14.0).color(pal.text))
            .id_salt(("ls_go_group", cx.salt, group.label()))
            .default_open(group != go::Group::Advanced)
            .show(ui, |ui| {
                for m in rows {
                    changed |= option_row(ui, o, m, cx, pal);
                }
                for l in layers {
                    changed |= layer_row(ui, o, l, cx, pal);
                }
            });
    }
    changed
}

/// A row is shown while its parent is on, and the lsfg-vk sub-settings only while
/// "Lodestar settings" is the profile (a named profile brings its own; compose ignores them then).
fn row_visible(eff: &go::Effective, m: &go::OptionMeta) -> bool {
    use OptionId as O;
    if !m.parent.is_none_or(|p| parent_on(eff, p)) {
        return false;
    }
    match m.id {
        O::LsfgMultiplier | O::LsfgFlowScale | O::LsfgPerformance => !lsfg_named_profile(eff),
        _ => true,
    }
}

/// The effective lsfg-vk profile is a name from its conf.toml (same filter as compose).
fn lsfg_named_profile(eff: &go::Effective) -> bool {
    eff.lsfg_profile.as_deref().map(str::trim).is_some_and(|p| !p.is_empty())
}

fn parent_on(eff: &go::Effective, parent: OptionId) -> bool {
    match parent {
        OptionId::Gamescope => eff.gamescope.is_on(),
        OptionId::Lsfg => eff.lsfg.is_on(),
        OptionId::WineFsr => eff.wine_fsr.is_on(),
        OptionId::Hdr => eff.hdr.is_on(),
        _ => true,
    }
}

const LABEL_W: f32 = 210.0;

fn option_row(ui: &mut Ui, o: &mut GameOptions, m: &go::OptionMeta, cx: &EditorCtx, pal: &Pal) -> bool {
    let unavailable = cx.caps.and_then(|c| machine_unavailable(m.id, c));
    let mut changed = false;
    ui.horizontal(|ui| {
        let indent = if m.parent.is_some() { 18.0 } else { 0.0 };
        ui.add_space(indent);
        let color = if unavailable.is_some() { pal.dim } else { pal.text };
        left_label(ui, LABEL_W - indent, RichText::new(m.label).color(color)).on_hover_text(m.help);
        ui.add_enabled_ui(unavailable.is_none(), |ui| {
            changed |= control(ui, o, m, cx);
        });
        // A value set before it became unavailable can still be dropped.
        if unavailable.is_some()
            && holds_value(o, m.id)
            && ui
                .small_button("Reset")
                .on_hover_text(if cx.global.is_some() { "Back to Default" } else { "Back to Auto" })
                .clicked()
        {
            changed |= reset_option(o, m.id);
        }
        if let Some(chip) = report_chip(cx.report, m.id, None) {
            chip.show(ui, pal);
        }
    });
    if m.kind == Kind::GsMore && unavailable.is_none() && more_open(ui, cx) {
        changed |= gs_more_panel(ui, o, cx, pal);
    }
    if let Some(why) = &unavailable {
        let hint = cx.caps.and_then(|c| go::caps::install_hint(m.id, c.distro));
        let text = match hint {
            Some(h) => format!("{why} — {h}"),
            None => why.clone(),
        };
        note(ui, &text, pal.dim);
    }
    if let Some((text, color)) = report_note(cx.report, m.id, None, pal) {
        note(ui, &text, color);
    }
    changed
}

/// A row label, left-aligned in a fixed-width column.
fn left_label(ui: &mut Ui, width: f32, text: RichText) -> egui::Response {
    ui.allocate_ui_with_layout(vec2(width, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(width);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

fn note(ui: &mut Ui, text: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + 8.0);
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).wrap());
    });
}

/// The label of the "Default" choice: per game it says what Settings gives.
fn default_label(cx: &EditorCtx, global: Option<Tri>) -> String {
    match (cx.global, global) {
        (None, _) => "Auto".into(),
        (Some(_), Some(Tri::On)) => "Default (On)".into(),
        (Some(_), Some(Tri::Off)) => "Default (Off)".into(),
        (Some(_), _) => "Default".into(),
    }
}

fn tri_ui(ui: &mut Ui, v: &mut Tri, default: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (val, label) in [(Tri::Default, default), (Tri::On, "On"), (Tri::Off, "Off")] {
            let b = egui::Button::selectable(*v == val, RichText::new(label).size(12.5)).min_size(vec2(44.0, 22.0));
            let r = ui.add(b);
            if r.clicked() && *v != val {
                *v = val;
                changed = true;
            }
        }
    })
    .response
    .on_hover_text(if default == "Auto" {
        "Auto: Lodestar adds nothing, the game and its runner decide. Off actively switches it off."
    } else {
        "Default follows Settings → Games."
    });
    changed
}

fn tri_field(o: &mut GameOptions, id: OptionId) -> Option<&mut Tri> {
    use OptionId as O;
    Some(match id {
        O::Wayland => &mut o.wayland,
        O::Hdr => &mut o.hdr,
        O::HdrForce => &mut o.hdr_force,
        O::Gamescope => &mut o.gamescope,
        O::GsAdaptiveSync => &mut o.gs_adaptive_sync,
        O::GsGrabCursor => &mut o.gs_grab_cursor,
        O::GsExposeWayland => &mut o.gs_expose_wayland,
        O::GsDisableWsi => &mut o.gs_disable_wsi,
        O::Gamemode => &mut o.gamemode,
        O::PerformanceProfile => &mut o.performance_profile,
        O::KeepAwake => &mut o.keep_awake,
        O::PreferDgpu => &mut o.prefer_dgpu,
        O::Ntsync => &mut o.ntsync,
        O::WineFsr => &mut o.wine_fsr,
        O::Fsr4Upgrade => &mut o.fsr4_upgrade,
        O::DlssUpgrade => &mut o.dlss_upgrade,
        O::XessUpgrade => &mut o.xess_upgrade,
        O::UpscalerIndicator => &mut o.upscaler_indicator,
        O::Lsfg => &mut o.lsfg,
        O::LsfgPerformance => &mut o.lsfg_performance,
        O::Mangohud => &mut o.mangohud,
        O::ObsCapture => &mut o.obs_capture,
        O::SdlInput => &mut o.sdl_input,
        O::AudioLatency => &mut o.audio_latency,
        O::ProtonLog => &mut o.proton_log,
        O::HdrWsiLayer => &mut o.hdr_wsi_layer,
        _ => return None,
    })
}

fn control(ui: &mut Ui, o: &mut GameOptions, m: &go::OptionMeta, cx: &EditorCtx) -> bool {
    let id = m.id;
    let salt = (cx.salt, id as u8);
    let global = cx.global;
    match m.kind {
        Kind::Tri => {
            let g = global.and_then(|g| tri_field(&mut g.clone(), id).copied());
            let label = default_label(cx, g);
            match tri_field(o, id) {
                Some(v) => tri_ui(ui, v, &label),
                None => false,
            }
        }
        Kind::WindowMode => {
            let inherited = global.and_then(|g| g.gs_window).unwrap_or_default();
            choice(
                ui,
                salt,
                &mut o.gs_window,
                inherited,
                cx.global.is_some(),
                &[
                    (WindowMode::Fullscreen, "Fullscreen"),
                    (WindowMode::Borderless, "Borderless window"),
                    (WindowMode::Windowed, "Window"),
                ],
            )
        }
        Kind::GsFilter => {
            let inherited = global.and_then(|g| g.gs_filter).unwrap_or_default();
            choice(
                ui,
                salt,
                &mut o.gs_filter,
                inherited,
                cx.global.is_some(),
                &[
                    (GsFilter::Auto, "Auto"),
                    (GsFilter::Fsr, "FSR"),
                    (GsFilter::Nis, "NIS"),
                    (GsFilter::Sgsr, "SGSR"),
                    (GsFilter::Linear, "Linear"),
                    (GsFilter::Nearest, "Nearest"),
                    (GsFilter::Pixel, "Pixel art"),
                ],
            )
        }
        Kind::GsScaler => {
            let inherited = global.and_then(|g| g.gs_scaler).unwrap_or_default();
            choice(
                ui,
                salt,
                &mut o.gs_scaler,
                inherited,
                cx.global.is_some(),
                &[
                    (GsScaler::Auto, "Auto"),
                    (GsScaler::Fit, "Fit"),
                    (GsScaler::Integer, "Integer"),
                    (GsScaler::Fill, "Fill"),
                    (GsScaler::Stretch, "Stretch"),
                ],
            )
        }
        Kind::Nvapi => {
            let mut changed = false;
            let inherited = global.map(|g| g.nvapi).unwrap_or_default();
            let def = if cx.global.is_some() { format!("Default ({})", nvapi_label(inherited)) } else { "Auto".into() };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (val, label) in [(Nvapi::Default, def.as_str()), (Nvapi::Off, "Off"), (Nvapi::Force, "Force")] {
                    let b = egui::Button::selectable(o.nvapi == val, RichText::new(label).size(12.5))
                        .min_size(vec2(44.0, 22.0));
                    if ui.add(b).clicked() && o.nvapi != val {
                        o.nvapi = val;
                        changed = true;
                    }
                }
            });
            changed
        }
        Kind::Res => res_ui(ui, salt, &mut o.gs_output, global.and_then(|g| g.gs_output), cx.global.is_some()),
        Kind::InternalRes => {
            internal_ui(ui, salt, &mut o.gs_internal, global.and_then(|g| g.gs_internal), cx.global.is_some())
        }
        Kind::Number { min, max } => {
            let (field, inherited): (&mut Option<u32>, Option<u32>) = match id {
                OptionId::FrameLimit => (&mut o.frame_limit, global.and_then(|g| g.frame_limit)),
                _ => return small_number(ui, o, id, global, min, max, cx.global.is_some()),
            };
            number_ui(
                ui,
                field,
                inherited,
                min,
                max,
                cx.global.is_some(),
                if id == OptionId::FrameLimit { " fps" } else { "" },
            )
        }
        Kind::Fraction => {
            let mut changed = false;
            let inherited = global.and_then(|g| g.lsfg_flow_scale);
            ui.horizontal(|ui| {
                let mut set = o.lsfg_flow_scale.is_some();
                let label = override_label(cx.global.is_some(), inherited.map(|v| format!("{v:.2}")), "1.00");
                if ui.checkbox(&mut set, label).changed() {
                    o.lsfg_flow_scale = set.then(|| inherited.unwrap_or(1.0));
                    changed = true;
                }
                if let Some(v) = &mut o.lsfg_flow_scale {
                    changed |=
                        ui.add(egui::DragValue::new(v).range(0.25..=1.0).speed(0.01).fixed_decimals(2)).changed();
                }
            });
            changed
        }
        Kind::Text if cx.global.is_some() => {
            // Per game / per library: an own value replaces the inherited one, and an empty own
            // value means "no extra arguments here" (unticked = follow Settings).
            let inherited = global.and_then(|g| g.gs_extra_args.clone()).filter(|t| !t.trim().is_empty());
            let mut changed = false;
            ui.horizontal(|ui| {
                let mut own = o.gs_extra_args.is_some();
                let label = override_label(true, inherited.clone(), "none");
                if ui.checkbox(&mut own, label).changed() {
                    set_own_text(&mut o.gs_extra_args, own, inherited.as_deref());
                    changed = true;
                }
                if let Some(text) = &mut o.gs_extra_args {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(text)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("empty = none")
                                .desired_width(200.0),
                        )
                        .changed();
                }
            });
            changed
        }
        Kind::Text => {
            let mut text = o.gs_extra_args.clone().unwrap_or_default();
            let hint = global.and_then(|g| g.gs_extra_args.clone()).unwrap_or_default();
            let r = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(if hint.is_empty() { "e.g. --hdr-itm-enabled".into() } else { hint })
                    .desired_width(240.0),
            );
            if r.changed() {
                o.gs_extra_args = Some(text).filter(|t| !t.trim().is_empty());
                true
            } else {
                false
            }
        }
        Kind::GsMore => {
            let open_id = more_id(cx);
            let mut open = more_open(ui, cx);
            let own = go::gamescope::count_set(o);
            let inherited = global.map(go::gamescope::count_set).unwrap_or(0);
            let summary = match (own, inherited, cx.global.is_some()) {
                (0, 0, _) => "none".to_string(),
                (n, 0, _) => format!("{n} on"),
                (0, i, true) => format!("{i} from Settings"),
                (n, i, _) => format!("{n} here, {i} from Settings"),
            };
            if ui.button(if open { "Hide list" } else { "Choose…" }).clicked() {
                open = !open;
                ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
            }
            ui.label(RichText::new(summary).size(12.0).weak());
            false
        }
        Kind::LsfgProfile => {
            let profiles: Vec<String> = cx
                .caps
                .and_then(|c| c.lsfg.as_ref())
                .map(|l| l.profiles.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default();
            let inherited = global.and_then(|g| g.lsfg_profile.clone());
            let show = |p: &Option<String>| match p.as_deref() {
                None if cx.global.is_some() => format!(
                    "Default ({})",
                    match inherited.as_deref() {
                        None | Some("") => "Lodestar settings",
                        Some(n) => n,
                    }
                ),
                None | Some("") => "Lodestar settings".to_string(),
                Some(n) => n.to_string(),
            };
            let mut changed = false;
            egui::ComboBox::from_id_salt(("ls_go_lsfgp", salt))
                .width(200.0)
                .selected_text(show(&o.lsfg_profile))
                .show_ui(ui, |ui| {
                    if cx.global.is_some() {
                        changed |= ui.selectable_value(&mut o.lsfg_profile, None, show(&None)).changed();
                    }
                    changed |=
                        ui.selectable_value(&mut o.lsfg_profile, Some(String::new()), "Lodestar settings").changed();
                    for p in profiles {
                        changed |= ui.selectable_value(&mut o.lsfg_profile, Some(p.clone()), p).changed();
                    }
                });
            changed
        }
    }
}

fn nvapi_label(v: Nvapi) -> &'static str {
    match v {
        Nvapi::Default => "Auto",
        Nvapi::Off => "Off",
        Nvapi::Force => "Force",
    }
}

/// Tick / untick the "Own value" box of a text option: ticking starts from the inherited text.
fn set_own_text(v: &mut Option<String>, own: bool, inherited: Option<&str>) {
    *v = own.then(|| inherited.unwrap_or_default().to_string());
}

/// The checkbox text of an optional value: per game "Set for this game (Settings: X)", globally
/// "Set (default X)".
fn override_label(per_game: bool, inherited: Option<String>, builtin: &str) -> String {
    if per_game {
        format!("Own value (Settings: {})", inherited.unwrap_or_else(|| builtin.to_string()))
    } else {
        format!("Set (default {builtin})")
    }
}

/// A choice from a fixed list; None = Default (per game) / the built-in value (global).
fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    salt: (&str, u8),
    v: &mut Option<T>,
    inherited: T,
    per_game: bool,
    items: &[(T, &str)],
) -> bool {
    let name = |t: T| items.iter().find(|(x, _)| *x == t).map(|(_, n)| *n).unwrap_or("?");
    let selected = match v {
        Some(t) => name(*t).to_string(),
        None if per_game => format!("Default ({})", name(inherited)),
        None => name(inherited).to_string(),
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(("ls_go_choice", salt)).width(200.0).selected_text(selected).show_ui(ui, |ui| {
        if per_game {
            changed |= ui.selectable_value(v, None, format!("Default ({})", name(inherited))).changed();
        }
        for (t, n) in items {
            changed |= ui.selectable_value(v, Some(*t), *n).changed();
        }
    });
    changed
}

fn res_ui(ui: &mut Ui, salt: (&str, u8), v: &mut Option<Res>, inherited: Option<Res>, per_game: bool) -> bool {
    let res_name = |r: Option<Res>| match r {
        None | Some(Res::Auto) => "Auto (this monitor)".to_string(),
        Some(Res::Custom { w, h }) => format!("{w} × {h}"),
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let selected = match v {
            None if per_game => format!("Default ({})", res_name(inherited)),
            _ => res_name(*v),
        };
        egui::ComboBox::from_id_salt(("ls_go_res", salt)).width(170.0).selected_text(selected).show_ui(ui, |ui| {
            if per_game {
                changed |= ui.selectable_value(v, None, format!("Default ({})", res_name(inherited))).changed();
            }
            changed |= ui.selectable_value(v, Some(Res::Auto), "Auto (this monitor)").changed();
            let custom = matches!(v, Some(Res::Custom { .. }));
            if ui.selectable_label(custom, "Custom…").clicked() && !custom {
                *v = Some(Res::Custom { w: 1920, h: 1080 });
                changed = true;
            }
        });
        if let Some(Res::Custom { w, h }) = v {
            changed |= ui.add(egui::DragValue::new(w).range(320..=15360)).changed();
            ui.label("×");
            changed |= ui.add(egui::DragValue::new(h).range(200..=8640)).changed();
        }
    });
    changed
}

fn internal_ui(
    ui: &mut Ui,
    salt: (&str, u8),
    v: &mut Option<InternalRes>,
    inherited: Option<InternalRes>,
    per_game: bool,
) -> bool {
    const PRESETS: [(u8, &str); 4] =
        [(77, "77 % (Ultra quality)"), (67, "67 % (Quality)"), (58, "58 % (Balanced)"), (50, "50 % (Performance)")];
    let name = |r: Option<InternalRes>| match r {
        None | Some(InternalRes::Native) => "Native".to_string(),
        Some(InternalRes::Percent(p)) => {
            PRESETS.iter().find(|(x, _)| *x == p).map(|(_, n)| n.to_string()).unwrap_or_else(|| format!("{p} %"))
        }
        Some(InternalRes::Custom { w, h }) => format!("{w} × {h}"),
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let selected = match v {
            None if per_game => format!("Default ({})", name(inherited)),
            _ => name(*v),
        };
        egui::ComboBox::from_id_salt(("ls_go_ires", salt)).width(190.0).selected_text(selected).show_ui(ui, |ui| {
            if per_game {
                changed |= ui.selectable_value(v, None, format!("Default ({})", name(inherited))).changed();
            }
            changed |= ui.selectable_value(v, Some(InternalRes::Native), "Native").changed();
            for (p, n) in PRESETS {
                changed |= ui.selectable_value(v, Some(InternalRes::Percent(p)), n).changed();
            }
            let custom = matches!(v, Some(InternalRes::Custom { .. }));
            if ui.selectable_label(custom, "Custom…").clicked() && !custom {
                *v = Some(InternalRes::Custom { w: 1280, h: 720 });
                changed = true;
            }
        });
        if let Some(InternalRes::Custom { w, h }) = v {
            changed |= ui.add(egui::DragValue::new(w).range(320..=15360)).changed();
            ui.label("×");
            changed |= ui.add(egui::DragValue::new(h).range(200..=8640)).changed();
        }
    });
    changed
}

fn number_ui(
    ui: &mut Ui,
    v: &mut Option<u32>,
    inherited: Option<u32>,
    min: u32,
    max: u32,
    per_game: bool,
    unit: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let mut set = v.is_some();
        let shown = |n: Option<u32>| match n {
            None | Some(0) if unit == " fps" => "no limit".to_string(),
            None => "–".to_string(),
            Some(n) => format!("{n}{unit}"),
        };
        let label = if per_game { format!("Own value (Settings: {})", shown(inherited)) } else { "Set".to_string() };
        if ui.checkbox(&mut set, label).changed() {
            *v = set.then(|| inherited.filter(|n| *n >= min).unwrap_or(if unit == " fps" { 60 } else { min }));
            changed = true;
        }
        if let Some(n) = v {
            changed |= ui.add(egui::DragValue::new(n).range(min..=max).suffix(unit)).changed();
        }
    });
    changed
}

fn small_number(
    ui: &mut Ui,
    o: &mut GameOptions,
    id: OptionId,
    global: Option<&GameOptions>,
    min: u32,
    max: u32,
    per_game: bool,
) -> bool {
    let (field, inherited) = match id {
        OptionId::GsSharpness => (&mut o.gs_sharpness, global.and_then(|g| g.gs_sharpness)),
        OptionId::WineFsrStrength => (&mut o.wine_fsr_strength, global.and_then(|g| g.wine_fsr_strength)),
        OptionId::LsfgMultiplier => (&mut o.lsfg_multiplier, global.and_then(|g| g.lsfg_multiplier)),
        _ => return false,
    };
    let mut wide = field.map(u32::from);
    let changed = number_ui(
        ui,
        &mut wide,
        inherited.map(u32::from),
        min,
        max,
        per_game,
        if id == OptionId::LsfgMultiplier { "×" } else { "" },
    );
    if changed {
        *field = wide.map(|n| n.min(255) as u8);
    }
    changed
}

// ------------------------------------------------------------------------------ Vulkan layers

/// Layers Lodestar handles through their own options (or that are part of the system).
const HANDLED_LAYERS: &[&str] = &[
    "VK_LAYER_MANGOHUD",
    "VK_LAYER_OBS_vkcapture",
    "VK_LAYER_LSFGVK",
    "VK_LAYER_LS_frame_generation",
    "VK_LAYER_FROG_gamescope_wsi",
    "VK_LAYER_hdr_wsi",
    "VK_LAYER_VALVE_",
    "VK_LAYER_MESA_device_select",
    "VK_LAYER_NV_optimus",
];

fn extra_layers(caps: Option<&Caps>) -> Vec<&go::LayerManifest> {
    let mut out: Vec<&go::LayerManifest> = vec![];
    for l in caps.map(|c| c.layers.as_slice()).unwrap_or_default() {
        if HANDLED_LAYERS.iter().any(|p| l.name.starts_with(p)) || out.iter().any(|x| x.name == l.name) {
            continue;
        }
        if l.enable_env.is_none() && l.disable_env.is_none() {
            continue;
        }
        out.push(l);
    }
    out
}

fn layer_row(ui: &mut Ui, o: &mut GameOptions, l: &go::LayerManifest, cx: &EditorCtx, pal: &Pal) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let label = if l.description.trim().is_empty() { l.name.as_str() } else { l.description.as_str() };
        left_label(ui, LABEL_W, RichText::new(label).color(pal.text)).on_hover_text(format!("Vulkan layer {}", l.name));
        let g = cx.global.and_then(|g| g.layers.get(&l.name).copied());
        let def = default_label(cx, g);
        let mut v = o.layers.get(&l.name).copied().unwrap_or_default();
        if tri_ui(ui, &mut v, &def) {
            if v == Tri::Default {
                o.layers.remove(&l.name);
            } else {
                o.layers.insert(l.name.clone(), v);
            }
            changed = true;
        }
        if let Some(chip) = report_chip(cx.report, OptionId::Layer, Some(&l.name)) {
            chip.show(ui, pal);
        }
    });
    if let Some((text, color)) = report_note(cx.report, OptionId::Layer, Some(&l.name), pal) {
        note(ui, &text, color);
    }
    changed
}

// --------------------------------------------------------------------------- availability

fn has_layer(caps: &Caps, prefix: &str) -> bool {
    caps.layers.iter().any(|l| l.name.starts_with(prefix))
}

/// Why an option can't work on this machine at all (None = it can, it might, or it depends on the
/// game's runner — the per-game report says). Unknown is never greyed: nothing before a real
/// probe, and no GPU gate without a GPU list.
pub(super) fn machine_unavailable(id: OptionId, caps: &Caps) -> Option<String> {
    use OptionId as O;
    if !caps.probed {
        return None;
    }
    let b = &caps.bins;
    let gamescope_ids = [
        O::Gamescope,
        O::GsWindow,
        O::GsOutput,
        O::GsInternal,
        O::GsFilter,
        O::GsSharpness,
        O::GsScaler,
        O::GsAdaptiveSync,
        O::GsGrabCursor,
        O::GsExposeWayland,
        O::GsDisableWsi,
        O::GsExtraArgs,
    ];
    if gamescope_ids.contains(&id) {
        if caps.nested_gamescope {
            return Some("Lodestar is already running inside Gamescope".into());
        }
        if b.gamescope.is_none() {
            return Some("Gamescope isn't installed".into());
        }
        return None;
    }
    let missing = |what: &str| Some(format!("{what} isn't installed"));
    let ext = |id: &str| caps.flatpak_ext.contains_key(id);
    // The GPUs a game can end up on: the default one, or the best one with "Use discrete GPU".
    let candidates: Vec<&go::Gpu> =
        [caps.rendering_gpu(false), caps.rendering_gpu(true)].into_iter().flatten().collect();
    let gpu_known = !candidates.is_empty();
    match id {
        O::Mangohud
            if b.mangohud.is_none()
                && b.mangoapp.is_none()
                && !has_layer(caps, "VK_LAYER_MANGOHUD")
                && !ext("org.freedesktop.Platform.VulkanLayer.MangoHud") =>
        {
            missing("MangoHud")
        }
        O::Gamemode if b.gamemoderun.is_none() => missing("GameMode"),
        O::PerformanceProfile if !performance_profile_available(b) => Some(if b.tlpctl.is_some() {
            "TLP alone can't switch the power profile for a game (needs power-profiles-daemon or CachyOS game-performance)".into()
        } else {
            "No power profile service (power-profiles-daemon) is installed".into()
        }),
        O::KeepAwake if b.systemd_inhibit.is_none() => missing("systemd-inhibit"),
        O::ObsCapture
            if b.obs_gamecapture.is_none()
                && !has_layer(caps, "VK_LAYER_OBS_vkcapture")
                && !ext("org.freedesktop.Platform.VulkanLayer.OBSVkCapture") =>
        {
            missing("obs-vkcapture")
        }
        O::Lsfg | O::LsfgProfile | O::LsfgMultiplier | O::LsfgFlowScale | O::LsfgPerformance if caps.lsfg.is_none() => {
            missing("lsfg-vk")
        }
        O::HdrWsiLayer if !has_layer(caps, "VK_LAYER_hdr_wsi") => missing("The HDR WSI layer (vk-hdr-layer)"),
        O::DlssUpgrade | O::Nvapi
            if gpu_known && caps.nvidia_version.is_none() && !candidates.iter().any(|g| g.vendor == 0x10de) =>
        {
            Some("Needs an NVIDIA GPU".into())
        }
        O::Fsr4Upgrade if gpu_known && !candidates.iter().any(|g| fsr4_gpu_or_unknown(g)) => {
            Some("Needs an AMD RDNA3 or newer GPU".into())
        }
        O::PreferDgpu => {
            if caps.gpus.len() < 2 {
                Some("This PC has one GPU".into())
            } else if caps.best_gpu().zip(caps.default_gpu()).is_some_and(|(best, def)| std::ptr::eq(best, def)) {
                Some("This PC already renders on its discrete GPU".into())
            } else {
                None
            }
        }
        O::Ntsync => match caps.ntsync {
            go::NtsyncState::Usable => None,
            go::NtsyncState::NotLoaded => Some("The ntsync kernel module isn't loaded".into()),
            go::NtsyncState::Missing => Some("This kernel has no NTSync".into()),
        },
        _ => None,
    }
}

/// What compose can use for "Performance power profile": CachyOS game-performance (which drives
/// power-profiles-daemon or TLP), else powerprofilesctl itself.
fn performance_profile_available(b: &go::Bins) -> bool {
    let ppc = b.powerprofilesctl.is_some();
    (b.game_performance.is_some() && (ppc || b.tlpctl.is_some())) || ppc
}

/// An AMD RDNA3+ GPU, or an AMD one whose generation couldn't be read.
fn fsr4_gpu_or_unknown(g: &go::Gpu) -> bool {
    match g.gc_ip {
        Some((major, _)) => major >= 11,
        None => g.vendor == 0x1002,
    }
}

// ------------------------------------------------------------------------------ reset

/// Drop `id`'s own value at this level (Default per game / Auto globally); true when one was set.
fn reset_option(o: &mut GameOptions, id: OptionId) -> bool {
    use OptionId as O;
    let before = o.clone();
    if let Some(t) = tri_field(o, id) {
        *t = Tri::Default;
    } else {
        match id {
            O::GsWindow => o.gs_window = None,
            O::GsOutput => o.gs_output = None,
            O::GsInternal => o.gs_internal = None,
            O::GsFilter => o.gs_filter = None,
            O::GsSharpness => o.gs_sharpness = None,
            O::GsScaler => o.gs_scaler = None,
            O::GsMore => o.gs_more.clear(),
            O::GsExtraArgs => o.gs_extra_args = None,
            O::FrameLimit => o.frame_limit = None,
            O::WineFsrStrength => o.wine_fsr_strength = None,
            O::LsfgProfile => o.lsfg_profile = None,
            O::LsfgMultiplier => o.lsfg_multiplier = None,
            O::LsfgFlowScale => o.lsfg_flow_scale = None,
            O::Nvapi => o.nvapi = Nvapi::Default,
            _ => {}
        }
    }
    *o != before
}

/// `id` has an own value at this level.
fn holds_value(o: &GameOptions, id: OptionId) -> bool {
    reset_option(&mut o.clone(), id)
}

// ---------------------------------------------------------------------------------- report

struct Chip {
    text: &'static str,
    good: bool,
}

impl Chip {
    fn show(&self, ui: &mut Ui, pal: &Pal) {
        let color = if self.good { pal.accent } else { pal.fav };
        ui.label(RichText::new(self.text).size(11.5).color(color));
    }
}

fn state_for<'a>(
    report: &'a [(OptionId, Option<String>, OptionState)],
    id: OptionId,
    layer: Option<&str>,
) -> Option<&'a OptionState> {
    report.iter().find(|(i, l, _)| *i == id && l.as_deref() == layer).map(|(_, _, s)| s)
}

fn report_chip(report: &[(OptionId, Option<String>, OptionState)], id: OptionId, layer: Option<&str>) -> Option<Chip> {
    Some(match state_for(report, id, layer)? {
        OptionState::Applied => Chip { text: "✓ applied", good: true },
        OptionState::AppliedImplicitly(_) => Chip { text: "✓ applied (needed)", good: true },
        OptionState::Suppressed(_) => Chip { text: "not applied", good: false },
        OptionState::Unavailable(_) => Chip { text: "not available", good: false },
        OptionState::NotApplicable => return None,
    })
}

fn report_note(
    report: &[(OptionId, Option<String>, OptionState)],
    id: OptionId,
    layer: Option<&str>,
    pal: &Pal,
) -> Option<(String, egui::Color32)> {
    match state_for(report, id, layer)? {
        OptionState::AppliedImplicitly(why) => Some((why.to_string(), pal.dim)),
        OptionState::Suppressed(why) => Some((why.clone(), pal.fav)),
        OptionState::Unavailable(why) => Some((why.clone(), pal.dim)),
        _ => None,
    }
}

// ------------------------------------------------------------------- More Gamescope options

fn more_id(cx: &EditorCtx) -> egui::Id {
    egui::Id::NULL.with(("ls_go_more_open", cx.salt))
}

fn more_open(ui: &Ui, cx: &EditorCtx) -> bool {
    ui.ctx().data(|d| d.get_temp(more_id(cx))).unwrap_or(false)
}

/// The searchable list of every extra gamescope flag, grouped; true when `o` changed.
fn gs_more_panel(ui: &mut Ui, o: &mut GameOptions, cx: &EditorCtx, pal: &Pal) -> bool {
    use go::gamescope::FlagGroup;
    let mut changed = false;
    let rows = go::gamescope::rows(cx.caps.and_then(|c| c.gamescope.as_ref()));
    let search_id = egui::Id::NULL.with(("ls_go_more_search", cx.salt));
    let mut search: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("🔍");
                if ui
                    .add(egui::TextEdit::singleline(&mut search).hint_text("Search Gamescope options").desired_width(260.0))
                    .changed()
                {
                    ui.ctx().data_mut(|d| d.insert_temp(search_id, search.clone()));
                }
                if cx.caps.and_then(|c| c.gamescope.as_ref()).is_none_or(|g| g.options.is_empty()) {
                    ui.label(RichText::new("(your Gamescope's own list couldn't be read; showing the common ones)").size(11.5).color(pal.dim));
                }
            });
            let q = search.trim().to_lowercase();
            for group in FlagGroup::ALL {
                let in_group: Vec<&go::gamescope::FlagRow> = rows
                    .iter()
                    .filter(|r| r.group == group)
                    .filter(|r| q.is_empty() || r.label.to_lowercase().contains(&q) || r.long.contains(&q) || r.help.to_lowercase().contains(&q))
                    .collect();
                if in_group.is_empty() {
                    continue;
                }
                let set_here = in_group.iter().filter(|r| o.gs_more.contains_key(&r.long)).count();
                let title = if set_here > 0 { format!("{} ({set_here})", group.label()) } else { group.label().to_string() };
                egui::CollapsingHeader::new(RichText::new(title).strong())
                    .id_salt(("ls_go_more_group", cx.salt, group.label()))
                    .default_open(!q.is_empty() || group != FlagGroup::Other && group != FlagGroup::Debug)
                    .open(if q.is_empty() { None } else { Some(true) })
                    .show(ui, |ui| {
                        for r in in_group {
                            changed |= flag_row(ui, o, r, cx, pal);
                        }
                    });
            }
            hint_line(ui, pal, "Default follows the level above (Settings → All games → library). Off switches a flag off that a broader level turned on.");
        });
    changed
}

fn hint_line(ui: &mut Ui, pal: &Pal, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal.dim)).wrap());
}

fn flag_row(ui: &mut Ui, o: &mut GameOptions, r: &go::gamescope::FlagRow, cx: &EditorCtx, pal: &Pal) -> bool {
    use go::gamescope::FlagKind;
    let mut changed = false;
    let inherited = cx.global.and_then(|g| g.gs_more.get(&r.long).cloned());
    let current = o.gs_more.get(&r.long).cloned();
    ui.horizontal(|ui| {
        left_label(ui, LABEL_W, RichText::new(&r.label).color(pal.text))
            .on_hover_text(format!("{}\n\n{}", r.help, r.long));
        // Default / On / Off, like every other option.
        let def = match (&inherited, cx.global.is_some()) {
            (_, false) => "Auto".to_string(),
            (Some(FlagSetting::On), true) => "Default (On)".into(),
            (Some(FlagSetting::Value(v)), true) => format!("Default ({v})"),
            (Some(FlagSetting::Off), true) | (None, true) => "Default".into(),
        };
        let mut t = match &current {
            None => Tri::Default,
            Some(FlagSetting::Off) => Tri::Off,
            Some(_) => Tri::On,
        };
        let before = t;
        if tri_ui(ui, &mut t, &def) && t != before {
            match t {
                Tri::Default => {
                    o.gs_more.remove(&r.long);
                }
                Tri::Off => {
                    o.gs_more.insert(r.long.clone(), FlagSetting::Off);
                }
                Tri::On => {
                    let start = match (r.kind, &inherited) {
                        (FlagKind::Switch, _) => FlagSetting::On,
                        (_, Some(FlagSetting::Value(v))) => FlagSetting::Value(v.clone()),
                        (FlagKind::Int { default, .. }, _) => FlagSetting::Value(default.to_string()),
                        (FlagKind::Decimal { default, .. }, _) => FlagSetting::Value(format!("{default}")),
                        (FlagKind::Choice(c), _) => FlagSetting::Value(c.first().copied().unwrap_or("").to_string()),
                        (FlagKind::Path | FlagKind::Text, _) => FlagSetting::Value(String::new()),
                    };
                    o.gs_more.insert(r.long.clone(), start);
                }
            }
            changed = true;
        }
        // The value, when it takes one and is on.
        let cur_val = match o.gs_more.get(&r.long) {
            Some(FlagSetting::Value(v)) => Some(v.clone()),
            Some(FlagSetting::On) if matches!(r.kind, FlagKind::Text) => Some(String::new()),
            _ => None,
        };
        if let Some(mut v) = cur_val {
            let edited = match r.kind {
                FlagKind::Switch => false,
                FlagKind::Int { min, max, default } => {
                    let mut n: i64 = v.trim().parse().unwrap_or(default);
                    let ch = ui.add(egui::DragValue::new(&mut n).range(min..=max)).changed();
                    if ch {
                        v = n.to_string();
                    }
                    ch
                }
                FlagKind::Decimal { min, max, default } => {
                    let mut n: f64 = v.trim().parse().unwrap_or(default);
                    let ch =
                        ui.add(egui::DragValue::new(&mut n).range(min..=max).speed(0.01).max_decimals(2)).changed();
                    if ch {
                        v = format!("{n}");
                    }
                    ch
                }
                FlagKind::Choice(choices) => {
                    let mut ch = false;
                    egui::ComboBox::from_id_salt(("ls_go_more_choice", cx.salt, r.long.as_str()))
                        .width(120.0)
                        .selected_text(v.clone())
                        .show_ui(ui, |ui| {
                            for c in choices {
                                if ui.selectable_label(v == *c, *c).clicked() && v != *c {
                                    v = c.to_string();
                                    ch = true;
                                }
                            }
                        });
                    ch
                }
                FlagKind::Path | FlagKind::Text => ui
                    .add(
                        egui::TextEdit::singleline(&mut v)
                            .font(egui::TextStyle::Monospace)
                            .hint_text(if r.kind == FlagKind::Text && r.group == go::gamescope::FlagGroup::Other {
                                "value (empty = just the switch)"
                            } else {
                                ""
                            })
                            .desired_width(180.0),
                    )
                    .changed(),
            };
            if edited {
                let setting = if v.trim().is_empty() && r.group == go::gamescope::FlagGroup::Other {
                    FlagSetting::On
                } else {
                    FlagSetting::Value(v)
                };
                o.gs_more.insert(r.long.clone(), setting);
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_options::{Bins, Gpu};
    use std::path::PathBuf;

    fn probed() -> Caps {
        Caps { probed: true, ..Default::default() }
    }

    fn bin(n: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("/usr/bin/{n}")))
    }

    fn gpu(card: &str, vendor: u16, driver: &str, boot: bool) -> Gpu {
        Gpu { card: card.into(), vendor, driver: driver.into(), boot_vga: boot, ..Default::default() }
    }

    #[test]
    fn nothing_is_greyed_before_a_real_probe() {
        let caps = Caps::default(); // nothing detected yet
        for m in go::OPTIONS {
            assert_eq!(machine_unavailable(m.id, &caps), None, "{:?}", m.id);
        }
        assert!(machine_unavailable(OptionId::Gamescope, &probed()).is_some());
    }

    #[test]
    fn performance_profile_matches_compose() {
        let with = |b: Bins| machine_unavailable(OptionId::PerformanceProfile, &Caps { bins: b, ..probed() });
        assert!(with(Bins::default()).is_some());
        assert!(
            with(Bins { tlpctl: bin("tlpctl"), ..Default::default() }).is_some(),
            "tlpctl alone: compose can't use it"
        );
        assert!(with(Bins { game_performance: bin("game-performance"), ..Default::default() }).is_some());
        assert!(with(Bins { powerprofilesctl: bin("powerprofilesctl"), ..Default::default() }).is_none());
        assert!(
            with(Bins { tlpctl: bin("tlpctl"), game_performance: bin("game-performance"), ..Default::default() })
                .is_none()
        );
    }

    #[test]
    fn mangohud_and_obs_count_layers_and_flatpak_extensions() {
        let caps = probed();
        assert!(machine_unavailable(OptionId::Mangohud, &caps).is_some());
        assert!(machine_unavailable(OptionId::ObsCapture, &caps).is_some());
        let mut flatpak = probed();
        flatpak.flatpak_ext.insert("org.freedesktop.Platform.VulkanLayer.MangoHud".into(), vec!["25.08".into()]);
        flatpak.flatpak_ext.insert("org.freedesktop.Platform.VulkanLayer.OBSVkCapture".into(), vec!["25.08".into()]);
        assert_eq!(machine_unavailable(OptionId::Mangohud, &flatpak), None);
        assert_eq!(machine_unavailable(OptionId::ObsCapture, &flatpak), None);
        let mut layer = probed();
        layer.layers.push(go::LayerManifest { name: "VK_LAYER_MANGOHUD_overlay_x86_64".into(), ..Default::default() });
        assert_eq!(machine_unavailable(OptionId::Mangohud, &layer), None);
    }

    #[test]
    fn gpu_gates_pass_when_the_gpus_are_unknown_or_tied() {
        use OptionId as O;
        let caps = probed(); // no GPU list
        for id in [O::DlssUpgrade, O::Nvapi, O::Fsr4Upgrade] {
            assert_eq!(machine_unavailable(id, &caps), None, "{id:?}");
        }
        // NVIDIA known only from the driver module.
        let caps =
            Caps { gpus: vec![gpu("card0", 0x8086, "i915", true)], nvidia_version: Some("595.12".into()), ..probed() };
        assert_eq!(machine_unavailable(O::DlssUpgrade, &caps), None);
        // Intel only: greyed.
        let caps = Caps { gpus: vec![gpu("card0", 0x8086, "i915", true)], ..probed() };
        assert!(machine_unavailable(O::DlssUpgrade, &caps).is_some());
        assert!(machine_unavailable(O::Fsr4Upgrade, &caps).is_some());
        assert!(machine_unavailable(O::PreferDgpu, &caps).is_some(), "one GPU");
        // AMD with an unreadable generation: not greyed.
        let caps = Caps { gpus: vec![gpu("card0", 0x1002, "amdgpu", true)], ..probed() };
        assert_eq!(machine_unavailable(O::Fsr4Upgrade, &caps), None);
        // nouveau dGPU listed first, Intel boot GPU last, nothing else to tell them apart.
        let caps =
            Caps { gpus: vec![gpu("card0", 0x10de, "nouveau", false), gpu("card1", 0x8086, "i915", true)], ..probed() };
        assert_eq!(machine_unavailable(O::PreferDgpu, &caps), None);
        // Equal on every key, the boot GPU listed last: the other card wins the tie.
        let caps =
            Caps { gpus: vec![gpu("card0", 0x8086, "i915", false), gpu("card1", 0x8086, "i915", true)], ..probed() };
        assert_eq!(machine_unavailable(O::PreferDgpu, &caps), None);
        // The boot GPU is the strongest one.
        let mut dgpu = gpu("card0", 0x1002, "amdgpu", true);
        dgpu.vram = Some(16 << 30);
        let caps = Caps { gpus: vec![dgpu, gpu("card1", 0x1002, "amdgpu", false)], ..probed() };
        assert!(machine_unavailable(O::PreferDgpu, &caps).is_some());
    }

    #[test]
    fn lsfg_settings_hide_under_a_named_profile() {
        let row = |id| go::meta(id);
        let mut o = GameOptions { lsfg: Tri::On, ..Default::default() };
        let eff = |g: &GameOptions, o: &GameOptions| go::effective(g, o);
        let none = GameOptions::default();
        assert!(row_visible(&eff(&none, &o), row(OptionId::LsfgMultiplier)));
        o.lsfg_profile = Some(String::new()); // "Lodestar settings"
        assert!(row_visible(&eff(&none, &o), row(OptionId::LsfgMultiplier)));
        o.lsfg_profile = Some("MyProfile".into());
        for id in [OptionId::LsfgMultiplier, OptionId::LsfgFlowScale, OptionId::LsfgPerformance] {
            assert!(!row_visible(&eff(&none, &o), row(id)), "{id:?}");
        }
        assert!(row_visible(&eff(&none, &o), row(OptionId::LsfgProfile)));
        // Inherited from Settings too.
        let global = GameOptions { lsfg_profile: Some("MyProfile".into()), ..Default::default() };
        let game = GameOptions { lsfg: Tri::On, ..Default::default() };
        assert!(!row_visible(&eff(&global, &game), row(OptionId::LsfgMultiplier)));
    }

    #[test]
    fn a_greyed_override_can_be_reset() {
        let mut o = GameOptions {
            gamescope: Tri::On,
            gs_window: Some(WindowMode::Windowed),
            nvapi: Nvapi::Force,
            frame_limit: Some(60),
            ..Default::default()
        };
        o.gs_more.insert("--rt".into(), FlagSetting::On);
        for id in [OptionId::Gamescope, OptionId::GsWindow, OptionId::Nvapi, OptionId::FrameLimit, OptionId::GsMore] {
            assert!(holds_value(&o, id), "{id:?}");
            assert!(reset_option(&mut o, id));
            assert!(!holds_value(&o, id), "{id:?}");
        }
        assert!(o.is_default());
        assert!(!reset_option(&mut o, OptionId::Mangohud), "nothing to reset");
    }

    #[test]
    fn every_option_can_be_reset() {
        // Every field away from its default: resetting each listed option gets back to Default.
        let mut o = GameOptions {
            gs_window: Some(WindowMode::Windowed),
            gs_output: Some(Res::default()),
            gs_internal: Some(InternalRes::default()),
            gs_filter: Some(GsFilter::default()),
            gs_sharpness: Some(5),
            gs_scaler: Some(GsScaler::default()),
            gs_extra_args: Some(String::new()),
            frame_limit: Some(0),
            wine_fsr_strength: Some(2),
            lsfg_profile: Some(String::new()),
            lsfg_multiplier: Some(3),
            lsfg_flow_scale: Some(0.5),
            nvapi: Nvapi::Off,
            ..Default::default()
        };
        o.gs_more.insert("--rt".into(), FlagSetting::Off);
        for m in go::OPTIONS {
            if let Some(t) = tri_field(&mut o, m.id) {
                *t = Tri::Off;
            }
        }
        for m in go::OPTIONS {
            assert!(holds_value(&o, m.id), "{:?} isn't reported as set", m.id);
            assert!(reset_option(&mut o, m.id), "{:?}", m.id);
        }
        assert!(o.is_default(), "left over: {o:?}");
    }

    #[test]
    fn per_game_extra_args_can_clear_the_global_ones() {
        let global = GameOptions { gs_extra_args: Some("--hdr-itm-enabled".into()), ..Default::default() };
        let mut game = GameOptions::default();
        assert_eq!(go::effective(&global, &game).gs_extra_args.as_deref(), Some("--hdr-itm-enabled"), "inherited");
        // Tick "Own value": starts from the inherited text; emptying it means none for this game.
        set_own_text(&mut game.gs_extra_args, true, Some("--hdr-itm-enabled"));
        assert_eq!(game.gs_extra_args.as_deref(), Some("--hdr-itm-enabled"));
        game.gs_extra_args = Some(String::new());
        assert_eq!(go::effective(&global, &game).gs_extra_args, None, "an empty own value blocks the global one");
        // Through a library layer as well.
        let lib = GameOptions { gs_extra_args: Some(String::new()), ..Default::default() };
        assert_eq!(go::effective(&global.overlay(&lib), &GameOptions::default()).gs_extra_args, None);
        // Untick: follow Settings again.
        set_own_text(&mut game.gs_extra_args, false, Some("--hdr-itm-enabled"));
        assert_eq!(game.gs_extra_args, None);
        // Nothing inherited: ticking starts empty, and the empty own value survives a save.
        set_own_text(&mut game.gs_extra_args, true, None);
        assert_eq!(game.gs_extra_args.as_deref(), Some(""));
        let back: GameOptions = serde_json::from_str(&serde_json::to_string(&game).unwrap()).unwrap();
        assert_eq!(back.gs_extra_args.as_deref(), Some(""));
        assert!(!back.is_default());
    }
}
