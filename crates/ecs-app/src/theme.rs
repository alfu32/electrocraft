//! Visual language (spec D10): SolidCraft's dark navy palette, Inter type,
//! rounded floating panels. Colours were sampled from SolidCraft 0.30.1.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    TextStyle, Visuals,
};
use std::sync::Arc;

/// Palette tokens.
pub mod c {
    use egui::Color32;
    pub const TITLEBAR: Color32 = Color32::from_rgb(0x1C, 0x1F, 0x26);
    pub const CANVAS: Color32 = Color32::from_rgb(0x2E, 0x34, 0x40);
    pub const GRID_MINOR: Color32 = Color32::from_rgb(0x34, 0x3A, 0x47);
    pub const GRID_MAJOR: Color32 = Color32::from_rgb(0x3D, 0x44, 0x53);
    pub const PANEL: Color32 = Color32::from_rgb(0x27, 0x2B, 0x34);
    pub const PANEL_BORDER: Color32 = Color32::from_rgb(0x34, 0x39, 0x44);
    pub const BAR: Color32 = Color32::from_rgb(0x21, 0x25, 0x2D);
    pub const NAV: Color32 = Color32::from_rgb(0x2B, 0x30, 0x3B);
    pub const TAB: Color32 = Color32::from_rgb(0x35, 0x3B, 0x47);
    pub const HOVER: Color32 = Color32::from_rgb(0x34, 0x3A, 0x46);
    pub const ACTIVE_TAB_BG: Color32 = Color32::from_rgb(0x2A, 0x46, 0x6E);
    pub const ACTIVE_TAB_TEXT: Color32 = Color32::from_rgb(0x9B, 0xC5, 0xF3);
    pub const ACCENT: Color32 = Color32::from_rgb(0x3A, 0x88, 0xEE);
    pub const ICON: Color32 = Color32::from_rgb(0x4E, 0xA1, 0xDC);
    pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xEA, 0xF2);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0xA4, 0xAB, 0xB8);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6E, 0x76, 0x84);
    pub const HOME_SIDE: Color32 = Color32::from_rgb(0x1E, 0x50, 0x80);
    pub const HOME_SIDE_SEL: Color32 = Color32::from_rgb(0x2E, 0x63, 0x95);
    pub const HOME_MAIN: Color32 = Color32::from_rgb(0x1F, 0x23, 0x2B);
    pub const CARD: Color32 = Color32::from_rgb(0x27, 0x2B, 0x34);
    pub const CARD_SEL: Color32 = Color32::from_rgb(0x1B, 0x30, 0x48);
    pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x5A, 0x4F);
    pub const WARN: Color32 = Color32::from_rgb(0xE8, 0xB3, 0x3C);
    pub const OK: Color32 = Color32::from_rgb(0x4C, 0xC3, 0x7A);
    /// Schematic ink on the dark canvas.
    pub const INK: Color32 = Color32::from_rgb(0xD5, 0xDB, 0xE6);
    pub const WIRE: Color32 = Color32::from_rgb(0x8F, 0xB8, 0xE0);
    pub const SELECT: Color32 = Color32::from_rgb(0x5C, 0xA8, 0xFF);
    pub const PIN: Color32 = Color32::from_rgb(0xF0, 0x8A, 0x4B);
    /// Scope trace colours.
    pub const TRACES: [Color32; 8] = [
        Color32::from_rgb(0xF5, 0xC2, 0x42),
        Color32::from_rgb(0x4E, 0xC9, 0xF0),
        Color32::from_rgb(0xF0, 0x6C, 0x9B),
        Color32::from_rgb(0x7C, 0xE0, 0x7A),
        Color32::from_rgb(0xC0, 0x8C, 0xF5),
        Color32::from_rgb(0xF5, 0x94, 0x4E),
        Color32::from_rgb(0x9C, 0xE5, 0xD6),
        Color32::from_rgb(0xE6, 0xE6, 0xE6),
    ];
}

/// Inter Regular, embedded.
pub const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
/// Inter SemiBold, embedded.
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
/// Noto Sans Mono, embedded (shortcuts and numbers).
pub const MONO: &[u8] = include_bytes!("../../../assets/fonts/NotoSansMono-Regular.ttf");

/// Family name of the semibold face.
pub const BOLD: &str = "Inter SemiBold";

/// A semibold font id.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// A regular font id.
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// A monospace font id.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Install fonts and visuals.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), Arc::new(FontData::from_static(INTER)));
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(FontData::from_static(INTER_SEMIBOLD)),
    );
    fonts
        .font_data
        .insert("noto-mono".into(), Arc::new(FontData::from_static(MONO)));
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "inter".into());
    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.insert(0, "noto-mono".into());
    let fallback: Vec<String> = fonts.families[&FontFamily::Proportional].clone();
    let mut semibold = vec!["inter-semibold".to_string()];
    semibold.extend(fallback.into_iter().filter(|f| f != "inter"));
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), semibold);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Small, regular(11.0)),
        (TextStyle::Body, regular(13.0)),
        (TextStyle::Button, regular(13.0)),
        (TextStyle::Heading, bold(26.0)),
        (TextStyle::Monospace, mono(12.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.menu_margin = Margin::same(6);

    let mut v = Visuals::dark();
    v.override_text_color = Some(c::TEXT);
    v.panel_fill = c::PANEL;
    v.window_fill = c::PANEL;
    v.extreme_bg_color = c::TITLEBAR;
    v.faint_bg_color = c::BAR;
    v.window_stroke = Stroke::new(1.0, c::PANEL_BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = c::ACTIVE_TAB_BG;
    v.selection.stroke = Stroke::new(1.0, c::ACTIVE_TAB_TEXT);
    v.hyperlink_color = c::ACCENT;
    let w = &mut v.widgets;
    for (state, fill) in [
        (&mut w.noninteractive, c::PANEL),
        (&mut w.inactive, c::TAB),
        (&mut w.hovered, c::HOVER),
        (&mut w.active, c::ACTIVE_TAB_BG),
        (&mut w.open, c::TAB),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(5);
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, c::TEXT);
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, c::PANEL_BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, c::TEXT_DIM);
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.hovered.bg_stroke = Stroke::new(1.0, c::PANEL_BORDER);
    style.visuals = v;
    ctx.set_global_style(style);
}

/// A rounded floating card, as SolidCraft's Browser and toolbars.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(c::PANEL)
        .stroke(Stroke::new(1.0, c::PANEL_BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(8))
        .shadow(Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(90),
        })
}
