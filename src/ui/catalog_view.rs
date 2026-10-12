use super::{
    theme::{self, Palette, ROW_HEIGHT, SPACE_LG, SPACE_MD, SPACE_SM},
    widgets, App,
};
use anime_downloader::{
    engine::Request,
    site::{catalog::SECTIONS, Kind, Series},
};
use eframe::egui::{self, vec2, Align, Layout, Margin, RichText, Sense, Stroke};

impl App {
    pub(super) fn catalog_view(&mut self, ctx: &egui::Context) {
        let ready = !self.catalog_loading && self.catalog_error.is_none();
        egui::TopBottomPanel::top("header")
            .frame(theme::header_frame(ctx))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ready {
                        ui.label(RichText::new(format!("{} series", self.catalog.len())).weak());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for (_, label, url) in SECTIONS.iter().rev() {
                            widgets::site_link(ui, label, url, &mut self.pending_link);
                        }
                        ui.label(RichText::new("Sources:").weak());
                    });
                });
                if !ready {
                    return;
                }
                section_summary(ui, &self.section_states());
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search anime…")
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(SPACE_MD as i8, SPACE_SM as i8)),
                );
            });
        let mut selected = None;
        let mut retry = false;
        egui::CentralPanel::default()
            .frame(theme::body_frame(ctx))
            .show(ctx, |ui| {
                if self.catalog_loading {
                    return loading_status(ui, &self.section_states());
                }
                if let Some(error) = &self.catalog_error {
                    retry = padded_notice(ui, error);
                    return;
                }
                let search = self.search.to_lowercase();
                let matches: Vec<&Series> = self
                    .catalog
                    .iter()
                    .filter(|series| series.title.to_lowercase().contains(&search))
                    .collect();
                if matches.is_empty() {
                    return widgets::status(ui, false, "No series match your search.");
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
                        for series in &matches[range] {
                            if series_row(ui, series) {
                                selected = Some((*series).clone());
                            }
                        }
                    });
            });
        if retry {
            self.catalog_loading = true;
            self.send(Request::LoadCatalog);
        }
        if let Some(series) = selected {
            self.select_series(series);
        }
    }
}

/// Why: sections load independently on the site, so the UI models each one on
/// its own; per-section engine events only need to feed `section_states`.
enum SectionState {
    Loading,
    Loaded(usize),
    Unavailable(String),
}

const SOURCE_HOST: &str = "cartoonsarea.cc";

fn section_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Sub => "Japanese with English subtitles",
        Kind::Dub => "English dub",
    }
}

impl SectionState {
    fn describe(&self) -> String {
        match self {
            Self::Loading => format!("loading anime list from {SOURCE_HOST}…"),
            Self::Loaded(count) => format!("{count} series"),
            Self::Unavailable(reason) => format!("unavailable: {reason}"),
        }
    }
}

impl App {
    /// Derived from the whole-catalog result until the engine reports sections
    /// separately; a section with a warning failed, the rest loaded.
    fn section_states(&self) -> Vec<(Kind, SectionState)> {
        SECTIONS
            .iter()
            .map(|(kind, label, _)| {
                let state = if self.catalog_loading {
                    SectionState::Loading
                } else if let Some(warning) = self.warnings.iter().find(|w| w.starts_with(label)) {
                    let reason = warning
                        .split_once("unavailable: ")
                        .map_or(warning.as_str(), |(_, reason)| reason);
                    SectionState::Unavailable(reason.to_owned())
                } else {
                    SectionState::Loaded(self.catalog.iter().filter(|s| s.kind == *kind).count())
                };
                (*kind, state)
            })
            .collect()
    }
}

fn loading_status(ui: &mut egui::Ui, states: &[(Kind, SectionState)]) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.3);
        ui.spinner();
        ui.add_space(SPACE_SM);
        for (kind, state) in states {
            ui.label(
                RichText::new(format!("{}: {}", section_name(*kind), state.describe())).weak(),
            );
        }
    });
}

fn section_summary(ui: &mut egui::Ui, states: &[(Kind, SectionState)]) {
    ui.horizontal_wrapped(|ui| {
        for (kind, state) in states {
            let text = format!("{}: {}", section_name(*kind), state.describe());
            if matches!(state, SectionState::Unavailable(_)) {
                ui.label(RichText::new(text).strong());
            } else {
                ui.label(RichText::new(text).weak());
            }
        }
    });
}

/// Error with a retry action, inset to line up with the screen padding.
pub(super) fn padded_notice(ui: &mut egui::Ui, error: &str) -> bool {
    let mut retry = false;
    ui.add_space(SPACE_LG);
    ui.horizontal(|ui| {
        ui.add_space(SPACE_LG);
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - SPACE_LG);
            retry = widgets::notice(ui, "Error", error, Some("Retry"));
        });
    });
    retry
}

/// Painted directly so the whole row, not just its text, is the click target.
fn series_row(ui: &mut egui::Ui, series: &Series) -> bool {
    let (rect, response) = widgets::row(ui, ROW_HEIGHT, Sense::click());
    let p = Palette::of(ui.visuals());
    let left = egui::pos2(rect.left() + SPACE_LG, rect.center().y);
    let badge_width = widgets::badge_at(ui, left, series.kind);
    let title_left = left.x + badge_width + SPACE_MD;
    let title = widgets::truncated(
        ui,
        &series.title,
        rect.right() - SPACE_LG * 2.0 - title_left,
    );
    let size = title.size();
    ui.painter().galley(
        egui::pos2(title_left, rect.center().y - size.y / 2.0),
        title,
        p.fg,
    );
    let tip = egui::pos2(rect.right() - SPACE_LG, rect.center().y);
    let stroke = Stroke::new(1.5, p.muted);
    ui.painter()
        .line_segment([tip + vec2(-5.0, -6.0), tip], stroke);
    ui.painter()
        .line_segment([tip + vec2(-5.0, 6.0), tip], stroke);
    response.clicked()
}
