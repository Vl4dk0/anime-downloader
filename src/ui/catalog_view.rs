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
        let listed = !self.catalog.is_empty();
        egui::TopBottomPanel::top("header")
            .frame(theme::header_frame(ctx))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if listed {
                        ui.label(RichText::new(format!("{} series", self.catalog.len())).weak());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        for (_, label, url) in SECTIONS.iter().rev() {
                            widgets::site_link(ui, label, url, &mut self.pending_link);
                        }
                        ui.label(RichText::new("Sources:").weak());
                    });
                });
                if !listed {
                    return;
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search anime…")
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(SPACE_MD as i8, SPACE_SM as i8)),
                );
                self.sections.status_line(ui);
            });
        let mut selected = None;
        let mut retry = false;
        egui::CentralPanel::default()
            .frame(theme::body_frame(ctx))
            .show(ctx, |ui| {
                if !listed {
                    if self.sections.any_loading() {
                        return widgets::status(ui, true, FETCHING);
                    }
                    retry = padded_notice(ui, &self.sections.unavailable().join("; "));
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
            self.sections = Sections::loading();
            self.send(Request::LoadCatalog);
        }
        if let Some(series) = selected {
            self.select_series(series);
        }
    }
}

const FETCHING: &str = "Fetching anime from cartoonsarea.cc…";

pub(super) enum SectionState {
    Loading,
    Loaded,
    Unavailable(String),
}

/// Fed only by per-section engine events, so a slow or dead source never
/// holds back the list from the source that already answered.
pub(super) struct Sections(Vec<(Kind, SectionState)>);

impl Sections {
    pub(super) fn loading() -> Self {
        Self(
            SECTIONS
                .iter()
                .map(|(kind, _, _)| (*kind, SectionState::Loading))
                .collect(),
        )
    }

    pub(super) fn set(&mut self, kind: Kind, state: SectionState) {
        if let Some(entry) = self.0.iter_mut().find(|(k, _)| *k == kind) {
            entry.1 = state;
        }
    }

    fn any_loading(&self) -> bool {
        self.0
            .iter()
            .any(|(_, state)| matches!(state, SectionState::Loading))
    }

    fn unavailable(&self) -> Vec<String> {
        self.0
            .iter()
            .filter_map(|(kind, state)| match state {
                SectionState::Unavailable(reason) => {
                    let label = SECTIONS
                        .iter()
                        .find(|(k, _, _)| k == kind)
                        .map_or("Catalog", |(_, label, _)| label);
                    Some(format!("{label} unavailable ({reason})"))
                }
                _ => None,
            })
            .collect()
    }

    /// Kept to one quiet line so it never competes with the list it sits above.
    fn status_line(&self, ui: &mut egui::Ui) {
        if self.any_loading() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(FETCHING).weak().small());
            });
            return;
        }
        let unavailable = self.unavailable();
        if !unavailable.is_empty() {
            ui.label(RichText::new(unavailable.join(" · ")).weak().small());
        }
    }
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
    let stroke = Stroke::new(1.5_f32, p.muted);
    ui.painter()
        .line_segment([tip + vec2(-5.0, -6.0), tip], stroke);
    ui.painter()
        .line_segment([tip + vec2(-5.0, 6.0), tip], stroke);
    response.clicked()
}
