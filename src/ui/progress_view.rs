use super::{App, FileState, LocalEvent, Screen};
use anime_downloader::engine::Request;
use eframe::egui;

impl App {
    pub(super) fn progress_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let done = self
            .job
            .files
            .values()
            .filter(|state| matches!(state, FileState::Done))
            .count();
        let skipped = self
            .job
            .files
            .values()
            .filter(|state| matches!(state, FileState::Skipped))
            .count();
        let failed = self
            .job
            .files
            .values()
            .filter(|state| matches!(state, FileState::Failed { .. }))
            .count();
        let completed = done + skipped + failed;
        let fraction = if self.job.total > 0 {
            completed as f32 / self.job.total as f32
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .text(format!("{completed} / {} completed", self.job.total)),
        );
        if !self.job.finished {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if self.job.cancelled {
                    "Cancelling…"
                } else {
                    "Discovering and downloading episodes…"
                });
            });
            if ui
                .add_enabled(!self.job.cancelled, egui::Button::new("Cancel"))
                .clicked()
            {
                self.job.cancelled = true;
                self.send(Request::Cancel);
            }
        } else {
            if self.job.cancelled {
                ui.label("Cancelled. Partial files are kept for resuming.");
            }
            ui.label(format!(
                "Downloaded: {done} · Skipped: {skipped} · Failed: {failed}"
            ));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.opening_folder, egui::Button::new("Open folder"))
                    .clicked()
                {
                    self.opening_folder = true;
                    let folder = self.job.folder.clone();
                    let sender = self.local_sender.clone();
                    let context = ctx.clone();
                    if let Some(runtime) = &self.runtime {
                        runtime.spawn_blocking(move || {
                            let result = opener::open(&folder)
                                .map_err(|error| format!("Open {}: {error}", folder.display()));
                            let _ = sender.send(LocalEvent::OpenResult(result));
                            context.request_repaint();
                        });
                    }
                }
                if ui.button("Back to catalog").clicked() {
                    self.screen = Screen::Catalog;
                }
            });
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for state in self.job.files.values() {
                if let FileState::Active(file) = state {
                    ui.label(&file.name);
                    let fraction = file
                        .size
                        .filter(|size| *size > 0)
                        .map(|size| file.bytes as f32 / size as f32)
                        .unwrap_or(0.0);
                    let total = file
                        .size
                        .map(|size| format!("{:.1}", size as f64 / 1_048_576.0))
                        .unwrap_or_else(|| "?".into());
                    let speed = if file.sampled_at.elapsed().as_secs() > 3 {
                        0.0
                    } else {
                        file.speed
                    };
                    ui.add(egui::ProgressBar::new(fraction).text(format!(
                        "{:.1} / {total} MB · {:.1} MB/s",
                        file.bytes as f64 / 1_048_576.0,
                        speed / 1_048_576.0
                    )));
                }
            }
            if failed > 0 {
                ui.collapsing(format!("Failures ({failed})"), |ui| {
                    for state in self.job.files.values() {
                        if let FileState::Failed { name, error } = state {
                            ui.label(name);
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                            ui.separator();
                        }
                    }
                });
                if ui
                    .add_enabled(self.job.finished, egui::Button::new("Retry failed"))
                    .clicked()
                {
                    self.job.finished = false;
                    self.job.cancelled = false;
                    self.send(Request::RetryFailed);
                }
            }
        });
    }
}
