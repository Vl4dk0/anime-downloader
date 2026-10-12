use super::App;
use anime_downloader::{engine::Request, site::Kind};
use eframe::egui;

impl App {
    pub(super) fn catalog_view(&mut self, ui: &mut egui::Ui) {
        if self.catalog_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading catalog…");
            });
            return;
        }
        if let Some(error) = &self.catalog_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
            if ui.button("Retry").clicked() {
                self.catalog_loading = true;
                self.send(Request::LoadCatalog);
            }
            return;
        }
        for warning in &self.warnings {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search anime…")
                .desired_width(f32::INFINITY),
        );
        let search = self.search.to_lowercase();
        let mut selected = None;
        let mut count = 0;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for series in self
                .catalog
                .iter()
                .filter(|series| series.title.to_lowercase().contains(&search))
            {
                count += 1;
                let kind = match series.kind {
                    Kind::Sub => "SUB",
                    Kind::Dub => "DUB",
                };
                if ui
                    .selectable_label(false, format!("[{kind}] {}", series.title))
                    .clicked()
                {
                    selected = Some(series.clone());
                }
            }
            if count == 0 {
                ui.label("No series match your search.");
            }
        });
        if let Some(series) = selected {
            self.select_series(series);
        }
    }
}
