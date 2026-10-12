//! One place that defines how the app looks: strictly black & white, following
//! the OS light/dark preference. Greys are only used for secondary text and
//! hover/pressed feedback, so no widget needs its own colour logic.

use eframe::egui::{
    self, style::ScrollStyle, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke,
    TextStyle, Theme, Visuals,
};

/// Spacing scale; every gap in the UI is one of these.
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;
pub const ROW_HEIGHT: f32 = 44.0;
/// Height of footer action buttons, shared so primary and secondary line up.
pub const BUTTON_HEIGHT: f32 = 40.0;

pub struct Palette {
    pub bg: Color32,
    pub fg: Color32,
    pub muted: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub line: Color32,
    pub outline: Color32,
    selection: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::BLACK,
    fg: Color32::WHITE,
    muted: Color32::from_gray(150),
    hover: Color32::from_gray(28),
    pressed: Color32::from_gray(56),
    line: Color32::from_gray(48),
    outline: Color32::from_gray(120),
    selection: Color32::from_gray(90),
};

const LIGHT: Palette = Palette {
    bg: Color32::WHITE,
    fg: Color32::BLACK,
    muted: Color32::from_gray(105),
    hover: Color32::from_gray(240),
    pressed: Color32::from_gray(215),
    line: Color32::from_gray(222),
    outline: Color32::from_gray(140),
    selection: Color32::from_gray(190),
};

impl Palette {
    pub fn of(visuals: &Visuals) -> &'static Palette {
        if visuals.dark_mode {
            &DARK
        } else {
            &LIGHT
        }
    }
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(Visuals::dark(), &DARK));
    ctx.set_visuals_of(Theme::Light, visuals(Visuals::light(), &LIGHT));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(26.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(16.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(16.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(15.0, FontFamily::Monospace),
            ),
        ]
        .into();
        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
        spacing.button_padding = egui::vec2(SPACE_MD, 7.0);
        spacing.interact_size.y = 32.0;
        spacing.icon_width = 20.0;
        spacing.icon_width_inner = 10.0;
        // A solid bar stays visible, so the scroll position is always readable.
        spacing.scroll = ScrollStyle::solid();
    });
}

fn visuals(base: Visuals, p: &Palette) -> Visuals {
    let radius = CornerRadius::same(6);
    let widget = |fill: Color32, stroke: Stroke| egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: stroke,
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0_f32, p.fg),
        expansion: 0.0,
    };
    let mut v = base;
    v.override_text_color = None;
    v.weak_text_color = Some(p.muted);
    v.hyperlink_color = p.fg;
    v.warn_fg_color = p.fg;
    v.error_fg_color = p.fg;
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.window_stroke = Stroke::new(1.0_f32, p.outline);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.extreme_bg_color = p.bg;
    v.text_edit_bg_color = Some(p.bg);
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.hover;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.5_f32, p.fg);
    v.text_cursor.stroke = Stroke::new(2.0_f32, p.fg);
    v.widgets.noninteractive = widget(p.bg, Stroke::new(1.0_f32, p.line));
    v.widgets.inactive = widget(p.bg, Stroke::new(1.0_f32, p.outline));
    v.widgets.hovered = widget(p.hover, Stroke::new(1.0_f32, p.fg));
    v.widgets.active = widget(p.pressed, Stroke::new(1.5_f32, p.fg));
    v.widgets.open = widget(p.hover, Stroke::new(1.0_f32, p.fg));
    v
}

pub fn header_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new()
        .fill(ctx.style().visuals.panel_fill)
        .inner_margin(Margin::symmetric(SPACE_LG as i8, SPACE_MD as i8))
}

/// Body panels have no margin so scroll areas reach the window edge and put
/// their scrollbar there; rows pad themselves.
pub fn body_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new().fill(ctx.style().visuals.panel_fill)
}
