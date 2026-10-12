use super::{App, Job, LocalEvent, Screen};
use anime_downloader::{download::safe_name, engine::Request};
use eframe::egui;

impl App {
    pub(super) fn series_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Back").clicked() {
            self.screen = Screen::Catalog;
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        ui.heading(&selection.series.title);
        if selection.loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading seasons…");
            });
            return;
        }
        if let Some(error) = &selection.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
            if ui.button("Retry").clicked() {
                let series = selection.series.clone();
                selection.loading = true;
                self.send(Request::LoadSeasons(series));
            }
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("Select all").clicked() {
                for (_, ticked) in &mut selection.seasons {
                    *ticked = true;
                }
            }
            if ui.button("Select none").clicked() {
                for (_, ticked) in &mut selection.seasons {
                    *ticked = false;
                }
            }
        });
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 120.0).max(80.0))
            .show(ui, |ui| {
                for (season, ticked) in &mut selection.seasons {
                    ui.checkbox(ticked, &season.label);
                }
            });
        ui.separator();
        ui.label(
            self.folder
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Choose a download folder".into()),
        );
        if ui
            .add_enabled(!self.choosing_folder, egui::Button::new("Choose folder…"))
            .clicked()
        {
            self.choosing_folder = true;
            let sender = self.local_sender.clone();
            let context = ctx.clone();
            let initial = self.folder.clone();
            if let Some(runtime) = &self.runtime {
                runtime.spawn_blocking(move || {
                    let mut dialog = rfd::FileDialog::new().set_title("Choose download folder");
                    if let Some(initial) = initial {
                        dialog = dialog.set_directory(initial);
                    }
                    let _ = sender.send(LocalEvent::Folder(dialog.pick_folder()));
                    context.request_repaint();
                });
            }
        }
        let enabled = self.folder.is_some() && selection.seasons.iter().any(|(_, ticked)| *ticked);
        if ui
            .add_enabled(enabled, egui::Button::new("Download"))
            .clicked()
        {
            let series = selection.series.clone();
            let seasons = selection
                .seasons
                .iter()
                .filter(|(_, ticked)| *ticked)
                .map(|(season, _)| season.clone())
                .collect();
            if let Some(folder) = self.folder.clone() {
                match safe_name(&series.title) {
                    Ok(name) => {
                        self.job = Job {
                            folder: folder.join(name),
                            ..Job::default()
                        };
                        self.screen = Screen::Progress;
                        self.error = None;
                        self.send(Request::StartDownload {
                            series,
                            seasons,
                            folder,
                        });
                    }
                    Err(error) => self.error = Some(format!("{error:#}")),
                }
            }
        }
    }
}
