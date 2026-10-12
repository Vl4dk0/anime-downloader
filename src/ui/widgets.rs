//! Small widgets shared by every screen, so that screens stay consistent and
//! only describe what they show.

use super::theme::{Palette, SPACE_LG, SPACE_MD};
use anime_downloader::site::Kind;
use eframe::egui::{
    self, text::LayoutJob, vec2, Align, Align2, Button, CursorIcon, FontId, Layout, Pos2, Rect,
    Response, RichText, Sense, Stroke, TextFormat, TextStyle, Ui,
};

pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Sub => "SUB",
        Kind::Dub => "DUB",
    }
}

/// Underlined text; colour alone would not mark links in a monochrome theme.
fn link(ui: &mut Ui, text: &str) -> Response {
    let color = ui.visuals().hyperlink_color;
    ui.add(
        egui::Label::new(RichText::new(text).color(color).underline())
            .selectable(false)
            .sense(Sense::click()),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
}

/// Queues `url` in `pending` for the app to open, since the UI closures only
/// borrow parts of the app and cannot reach the runtime that opens it.
pub fn site_link(ui: &mut Ui, label: &str, url: &str, pending: &mut Option<String>) {
    if link(ui, label).on_hover_text(url).clicked() {
        *pending = Some(url.to_owned());
    }
}

fn badge_galley(ui: &Ui, kind: Kind) -> std::sync::Arc<egui::Galley> {
    let color = Palette::of(ui.visuals()).fg;
    ui.painter()
        .layout_no_wrap(kind_label(kind).into(), FontId::monospace(12.0), color)
}

fn paint_badge(ui: &Ui, rect: Rect, galley: std::sync::Arc<egui::Galley>) {
    let p = Palette::of(ui.visuals());
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, p.outline),
        egui::StrokeKind::Inside,
    );
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, p.fg);
}

fn badge_size(galley: &egui::Galley) -> egui::Vec2 {
    galley.size() + vec2(12.0, 4.0)
}

/// Text identifies the source without depending on colour in the monochrome theme.
pub fn badge(ui: &mut Ui, kind: Kind) {
    let galley = badge_galley(ui, kind);
    let (rect, _) = ui.allocate_exact_size(badge_size(&galley), Sense::hover());
    paint_badge(ui, rect, galley);
}

/// Paints a badge vertically centred at `left_center` and returns its width,
/// for rows that are drawn directly with the painter.
pub fn badge_at(ui: &Ui, left_center: Pos2, kind: Kind) -> f32 {
    let galley = badge_galley(ui, kind);
    let size = badge_size(&galley);
    paint_badge(
        ui,
        Rect::from_min_size(left_center - vec2(0.0, size.y / 2.0), size),
        galley,
    );
    size.x
}

/// Painted rows need a bounded galley so long titles leave room for other columns.
pub fn truncated(ui: &Ui, text: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let p = Palette::of(ui.visuals());
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat::simple(TextStyle::Body.resolve(ui.style()), p.fg),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// Thin determinate bar; drawn by hand because egui's own draws its label in
/// the selection colour, which is unreadable on the empty part in this theme.
pub fn progress_bar(ui: &Ui, rect: Rect, fraction: f32) {
    let p = Palette::of(ui.visuals());
    let radius = rect.height() / 2.0;
    ui.painter().rect_filled(rect, radius, p.line);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, radius, p.fg);
}

/// Inverted (filled) button marking the one action a screen is for.
pub fn primary_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    let p = Palette::of(ui.visuals());
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, fill) in [
            (&mut widgets.inactive, p.fg),
            (&mut widgets.hovered, p.muted),
            (&mut widgets.active, p.muted),
        ] {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, p.fg);
        }
        ui.add_enabled(
            enabled,
            Button::new(RichText::new(text).strong().color(p.bg)).min_size(vec2(180.0, 40.0)),
        )
    })
    .inner
}

pub fn secondary_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    ui.add_enabled(enabled, Button::new(text).min_size(vec2(0.0, 40.0)))
}

pub fn small_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(Button::new(RichText::new(text).size(14.0)))
}

/// Bordered message; the border and a bold lead word carry the severity that
/// colour would in a coloured theme. Returns whether the action was clicked.
pub fn notice(ui: &mut Ui, lead: &str, text: &str, action: Option<&str>) -> bool {
    let p = Palette::of(ui.visuals());
    let mut clicked = false;
    egui::Frame::new()
        .stroke(Stroke::new(1.5, p.fg))
        .corner_radius(6.0)
        .inner_margin(SPACE_MD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(lead).strong());
                ui.label(text);
            });
            if let Some(action) = action {
                clicked = ui.button(action).clicked();
            }
        });
    clicked
}

pub fn status(ui: &mut Ui, spinner: bool, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.3);
        if spinner {
            ui.spinner();
        }
        ui.label(RichText::new(text).weak());
    });
}

/// Shared hit areas and separators keep painted rows consistent across screens.
pub fn row(ui: &mut Ui, height: f32, sense: Sense) -> (Rect, Response) {
    let p = Palette::of(ui.visuals());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            Some(p.pressed)
        } else if response.contains_pointer() {
            Some(p.hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, 0.0, fill);
        }
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
    }
    (rect, response)
}

/// Content area of a row, inset by the screen padding.
pub fn row_content(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui)) {
    let inner = rect.shrink2(vec2(SPACE_LG, 0.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
        add,
    );
}

pub fn paint_text_right(ui: &Ui, right_center: Pos2, text: &str, font: FontId, muted: bool) {
    let p = Palette::of(ui.visuals());
    let color = if muted { p.muted } else { p.fg };
    ui.painter()
        .text(right_center, Align2::RIGHT_CENTER, text, font, color);
}
