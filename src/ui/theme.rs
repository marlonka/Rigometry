//! Design tokens: colors, type and widget styling for both themes.
//!
//! Components read roles from [`Palette`] instead of raw colors, so a theme
//! change is one lookup and the two themes keep the same geometry.

use crate::state::Settings;
use eframe::egui::{
    self, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak,
    Id, Margin, Shadow, Stroke, TextStyle, Theme, ThemePreference, Ui, ViewportCommand,
    epaint::text::VariationCoords, style::HandleShape, style::ScrollStyle, vec2,
};
use std::path::PathBuf;

/// The interface's 100% is 125% of egui's logical size.
pub const BASE_SCALE: f32 = 1.25;
/// Minimum window in physical pixels at 100%.
pub const MIN_WINDOW: [f32; 2] = [1080., 720.];

/// Semantic color roles. Dark and light values are kept side by side.
pub struct Palette {
    /// Behind the sidebar and around the content canvas.
    pub window: Color32,
    /// The rounded content area that holds each page.
    pub canvas: Color32,
    /// Cards and grouped surfaces on the canvas.
    pub surface: Color32,
    /// Buttons, inputs and other controls at rest.
    pub control: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// Selected navigation, segments and rows.
    pub selected: Color32,
    /// Hairlines around cards and the canvas.
    pub border: Color32,
    /// Control outlines; stronger than decorative borders.
    pub control_border: Color32,
    pub divider: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Captions and axis labels; still at least 4.5:1 on surfaces.
    pub faint: Color32,
    /// Scrollbar handles, empty meter tracks and chart grid.
    pub track: Color32,
    pub primary: Color32,
    pub on_primary: Color32,
    pub warn: Color32,
    pub error: Color32,
    /// One accent for "something new": update available, scanning.
    pub accent: Color32,
    /// Data series, checked against both surfaces for color-vision deficiency.
    pub cpu: Color32,
    pub memory: Color32,
    pub gpu: Color32,
    pub temperature: Color32,
    /// Size map tiles: folders, then file kinds in [`crate::search::Kind`] order.
    pub tiles: [Color32; 7],
    pub tile_text: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const DARK: Palette = Palette {
    window: rgb(0x0b0b0c),
    canvas: rgb(0x131315),
    surface: rgb(0x19191c),
    control: rgb(0x1f1f23),
    hover: rgb(0x27272c),
    pressed: rgb(0x303036),
    selected: rgb(0x26262b),
    border: rgb(0x242428),
    control_border: rgb(0x34343b),
    divider: rgb(0x232327),
    text: rgb(0xededef),
    muted: rgb(0xa1a1aa),
    faint: rgb(0x8a8a93),
    track: rgb(0x2c2c32),
    primary: rgb(0xededef),
    on_primary: rgb(0x0b0b0c),
    warn: rgb(0xe8b04b),
    error: rgb(0xf07171),
    accent: rgb(0x3987e5),
    cpu: rgb(0x3987e5),
    memory: rgb(0x199e70),
    gpu: rgb(0xd95926),
    temperature: rgb(0x9085e9),
    tiles: [
        rgb(0x34363d),
        rgb(0x2f5f8f),
        rgb(0x8a4a2c),
        rgb(0x1f6e52),
        rgb(0x5f5499),
        rgb(0x86536a),
        rgb(0x4b4f58),
    ],
    tile_text: rgb(0xf4f4f5),
};

pub const LIGHT: Palette = Palette {
    window: rgb(0xf4f4f5),
    canvas: rgb(0xffffff),
    surface: rgb(0xfafafa),
    control: rgb(0xffffff),
    hover: rgb(0xf0f0f2),
    pressed: rgb(0xe6e6e9),
    selected: rgb(0xe9e9ec),
    border: rgb(0xe6e6e9),
    control_border: rgb(0xd4d4d8),
    divider: rgb(0xececef),
    text: rgb(0x18181b),
    muted: rgb(0x52525b),
    faint: rgb(0x6b6b74),
    track: rgb(0xe4e4e7),
    primary: rgb(0x18181b),
    on_primary: rgb(0xffffff),
    warn: rgb(0xa55a00),
    error: rgb(0xc42b2b),
    accent: rgb(0x2a78d6),
    cpu: rgb(0x2a78d6),
    memory: rgb(0x16946a),
    gpu: rgb(0xe05f2b),
    temperature: rgb(0x6250d6),
    tiles: [
        rgb(0xdcdce1),
        rgb(0xc4d9f3),
        rgb(0xf4d0bd),
        rgb(0xbfe3d1),
        rgb(0xd8d2f4),
        rgb(0xf1cbd9),
        rgb(0xe8e8eb),
    ],
    tile_text: rgb(0x18181b),
};

pub fn palette(ui: &Ui) -> &'static Palette {
    palette_for(ui.visuals().dark_mode)
}

pub fn palette_for(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

/// Named weights of the bundled variable Inter. Body text uses the regular
/// proportional family.
pub mod weight {
    pub const MEDIUM: &str = "Inter Medium";
    pub const SEMIBOLD: &str = "Inter Semibold";
    /// Semibold drawn at its display optical size, for titles and large numbers.
    pub const DISPLAY: &str = "Inter Display";
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(weight::MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(weight::SEMIBOLD.into()))
}
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(weight::DISPLAY.into()))
}
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Inter for every script it covers, then system and egui fonts as fallbacks
/// for file names in other scripts. System fonts are read, never redistributed.
fn fonts() -> FontDefinitions {
    static INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Rigometry.ttf");
    let mut fonts = FontDefinitions::default();
    let mut fallbacks = Vec::new();
    let windows_fonts = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("Fonts"))
        .unwrap_or_else(|| PathBuf::from("C:/Windows/Fonts"));
    // Segoe UI adds Arabic, Hebrew, Armenian and Georgian; the others add CJK.
    // Each covers different scripts (Hangul is only in the Korean fonts), so
    // every one found is added.
    for candidate in [
        windows_fonts.join("segoeui.ttf"),
        windows_fonts.join("msgothic.ttc"),
        windows_fonts.join("msyh.ttc"),
        windows_fonts.join("malgun.ttf"),
        PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc"),
        PathBuf::from("/System/Library/Fonts/AppleSDGothicNeo.ttc"),
    ] {
        if let Ok(font) = std::fs::read(&candidate) {
            let name = candidate.to_string_lossy().into_owned();
            fonts
                .font_data
                .insert(name.clone(), FontData::from_owned(font).into());
            fallbacks.push(name);
        }
    }
    let defaults = fonts.families[&FontFamily::Proportional].clone();
    let monospace = fonts.families[&FontFamily::Monospace].clone();
    for (family, wght, opsz) in [
        (FontFamily::Proportional, 400., 14.),
        (FontFamily::Name(weight::MEDIUM.into()), 500., 14.),
        (FontFamily::Name(weight::SEMIBOLD.into()), 600., 14.),
        (FontFamily::Name(weight::DISPLAY.into()), 600., 28.),
    ] {
        let name = format!("Inter {wght} {opsz}");
        fonts.font_data.insert(
            name.clone(),
            FontData::from_static(INTER)
                .tweak(FontTweak {
                    coords: VariationCoords::new([(b"wght", wght), (b"opsz", opsz)]),
                    ..FontTweak::default()
                })
                .into(),
        );
        let list = std::iter::once(name)
            .chain(fallbacks.iter().cloned())
            .chain(defaults.iter().cloned())
            .collect();
        fonts.families.insert(family, list);
    }
    fonts.families.insert(
        FontFamily::Monospace,
        monospace.into_iter().chain(fallbacks).collect(),
    );
    fonts
}

/// Applies theme, scale and widget styling. Safe to call every time a
/// setting changes.
pub fn apply(ctx: &Context, settings: &Settings) {
    install_fonts(ctx);
    // Ctrl/⌘ + plus/minus would zoom past the scale setting and the minimum
    // window size derived from it; the setting is the one zoom control.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(
        vec2(MIN_WINDOW[0], MIN_WINDOW[1]) / (BASE_SCALE * settings.scale),
    ));
    ctx.set_theme(match settings.theme.as_str() {
        "Light" => ThemePreference::Light,
        "System" => ThemePreference::System,
        _ => ThemePreference::Dark,
    });
    ctx.set_zoom_factor(BASE_SCALE * settings.scale);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            let p = palette_for(theme == Theme::Dark);
            let spacing = &mut style.spacing;
            spacing.item_spacing = vec2(8., 6.);
            spacing.button_padding = vec2(12., 5.);
            spacing.interact_size = vec2(32., 30.);
            spacing.icon_width = 16.;
            spacing.icon_width_inner = 10.;
            spacing.icon_spacing = 8.;
            spacing.indent = 20.;
            spacing.menu_margin = Margin::same(6);
            spacing.window_margin = Margin::same(10);
            spacing.combo_height = 320.;
            // A solid bar keeps its own gutter, so right-aligned values never
            // sit beneath it; its track is painted in the canvas color.
            spacing.scroll = ScrollStyle {
                bar_width: 6.,
                bar_inner_margin: 12.,
                handle_min_length: 32.,
                ..ScrollStyle::solid()
            };
            spacing.scroll.fade.size = 16.;
            style.animation_time = if settings.reduce_motion { 0. } else { 0.12 };
            // Reduced motion jumps to a scroll target instead of gliding.
            style.scroll_animation = if settings.reduce_motion {
                egui::style::ScrollAnimation::none()
            } else {
                egui::style::ScrollAnimation::default()
            };
            for (text, font) in [
                (TextStyle::Body, regular(14.)),
                (TextStyle::Button, medium(14.)),
                (TextStyle::Small, regular(12.)),
                (TextStyle::Heading, display(24.)),
                (TextStyle::Monospace, FontId::monospace(13.)),
            ] {
                style.text_styles.insert(text, font);
            }
            let v = &mut style.visuals;
            v.override_text_color = Some(p.text);
            v.weak_text_color = Some(p.muted);
            v.hyperlink_color = p.text;
            v.panel_fill = p.window;
            v.faint_bg_color = p.surface;
            // Scroll tracks blend into popups; the page track into the canvas
            // (`page_scroll`).
            v.extreme_bg_color = if theme == Theme::Dark {
                rgb(0x1c1c20)
            } else {
                p.canvas
            };
            v.text_edit_bg_color = Some(p.control);
            v.code_bg_color = p.control;
            v.warn_fg_color = p.warn;
            v.error_fg_color = p.error;
            v.window_fill = v.extreme_bg_color;
            v.window_stroke = Stroke::new(1.0_f32, p.control_border);
            v.window_corner_radius = CornerRadius::same(12);
            v.menu_corner_radius = CornerRadius::same(10);
            let shadow = Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(if theme == Theme::Dark { 110 } else { 28 }),
            };
            v.window_shadow = shadow;
            v.popup_shadow = shadow;
            v.selection.bg_fill = p.selected;
            v.selection.stroke = Stroke::new(1.5_f32, p.text);
            v.text_cursor.stroke = Stroke::new(2.0_f32, p.text);
            v.slider_trailing_fill = true;
            v.handle_shape = HandleShape::Circle;
            v.indent_has_left_vline = false;
            v.collapsing_header_frame = false;
            v.striped = false;
            // `bg_fill` paints scroll handles and checkbox boxes,
            // `weak_bg_fill` buttons and combo boxes.
            let w = &mut v.widgets;
            w.noninteractive.bg_fill = p.surface;
            w.noninteractive.weak_bg_fill = p.surface;
            w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.divider);
            w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
            w.inactive.bg_fill = p.track;
            w.inactive.weak_bg_fill = p.control;
            w.inactive.bg_stroke = Stroke::new(1.0_f32, p.control_border);
            w.inactive.fg_stroke = Stroke::new(1.5_f32, p.text);
            w.hovered.bg_fill = p.control_border;
            w.hovered.weak_bg_fill = p.hover;
            w.hovered.bg_stroke = Stroke::new(1.0_f32, p.control_border);
            w.hovered.fg_stroke = Stroke::new(1.5_f32, p.text);
            w.active.bg_fill = p.muted;
            w.active.weak_bg_fill = p.pressed;
            w.active.bg_stroke = Stroke::new(1.0_f32, p.control_border);
            w.active.fg_stroke = Stroke::new(1.5_f32, p.text);
            w.open = w.hovered;
            for w in [
                &mut w.noninteractive,
                &mut w.inactive,
                &mut w.hovered,
                &mut w.active,
                &mut w.open,
            ] {
                w.corner_radius = CornerRadius::same(8);
                w.expansion = 0.;
            }
        });
    }
}

/// Registers the bundled fonts once per context. They take effect at the
/// start of the next frame.
pub fn install_fonts(ctx: &Context) {
    let installed = Id::new("rigometry-fonts");
    if !ctx.data(|d| d.get_temp::<bool>(installed).unwrap_or(false)) {
        ctx.set_fonts(fonts());
        ctx.data_mut(|d| d.insert_temp(installed, true));
    }
}

/// Whether the weights used by components are available in this frame.
#[cfg(test)]
pub fn fonts_ready(ctx: &Context) -> bool {
    ctx.fonts(|fonts| {
        fonts
            .definitions()
            .families
            .contains_key(&FontFamily::Name(weight::MEDIUM.into()))
    })
}

/// Mixes `color` toward `base` by `amount` (0: base, 1: color), in sRGB.
pub fn mix(base: Color32, color: Color32, amount: f32) -> Color32 {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        channel(base.r(), color.r()),
        channel(base.g(), color.g()),
        channel(base.b(), color.b()),
    )
}
