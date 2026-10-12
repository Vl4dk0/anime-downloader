use super::{
    catalog_view::padded_notice,
    theme::{self, ROW_HEIGHT},
    widgets, App, Job, LocalEvent, Screen,
};
use anime_downloader::{download::safe_name, engine::Request};
use eframe::egui::{self, Align, Layout, RichText, Sense};

impl App {
    pub(super) fn series_view(&mut self, ctx: &egui::Context) {
        let Some(selection) = &mut self.selection else {
            self.screen = Screen::Catalog;
            return;
        };
        let mut back = false;
        let ticked_count = selection.seasons.iter().filter(|(_, t)| *t).count();
        egui::TopBottomPanel::top("header")
            .frame(theme::header_frame(ctx))
            .show(ctx, |ui| {
                back = widgets::small_button(ui, "Back").clicked();
                ui.horizontal(|ui| {
                    widgets::badge(ui, selection.series.kind);
                    ui.add(
                        egui::Label::new(RichText::new(&selection.series.title).heading().strong())
                            .truncate(),
                    );
                });
                ui.horizontal(|ui| {
                    if !selection.loading && selection.error.is_none() {
                        ui.label(
                            RichText::new(format!(
                                "{ticked_count} of {} seasons selected",
                                selection.seasons.len()
                            ))
                            .weak(),
                        );
                    }
                    widgets::site_link(
                        ui,
                        "Series page",
                        selection.series.url.as_str(),
                        &mut self.pending_link,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if selection.loading || selection.error.is_some() {
                            return;
                        }
                        if widgets::small_button(ui, "Select none").clicked() {
                            selection.seasons.iter_mut().for_each(|(_, t)| *t = false);
                        }
                        if widgets::small_button(ui, "Select all").clicked() {
                            selection.seasons.iter_mut().for_each(|(_, t)| *t = true);
                        }
                    });
                });
            });
        if back {
            self.screen = Screen::Catalog;
            return;
        }
        let ready = !selection.loading && selection.error.is_none();
        let mut start = false;
        let mut choose = false;
        if ready {
            egui::TopBottomPanel::bottom("footer")
                .frame(theme::header_frame(ctx))
                .show(ctx, |ui| {
                    // One fixed-height row so every control shares a centre line and
                    // the panel margin is the only padding above and below it.
                    let size = egui::vec2(ui.available_width(), theme::BUTTON_HEIGHT);
                    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
                        let enabled = self.folder.is_some() && ticked_count > 0;
                        let label = match ticked_count {
                            0 => "Download".to_owned(),
                            1 => "Download 1 season".to_owned(),
                            n => format!("Download {n} seasons"),
                        };
                        let download = widgets::primary_button(ui, enabled, &label);
                        if !enabled {
                            let hint = if self.folder.is_none() {
                                "Choose a folder to continue"
                            } else {
                                "Select at least one season"
                            };
                            download.on_disabled_hover_text(hint);
                        } else if download.clicked() {
                            start = true;
                        }
                        choose =
                            widgets::secondary_button(ui, !self.choosing_folder, "Choose folder…")
                                .clicked();
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let folder = self
                                .folder
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "no folder chosen".into());
                            ui.add(egui::Label::new(RichText::new("Save to").weak()));
                            ui.add(egui::Label::new(&folder).truncate())
                                .on_hover_text(&folder);
                        });
                    });
                });
        }
        let mut retry = false;
        egui::CentralPanel::default()
            .frame(theme::body_frame(ctx))
            .show(ctx, |ui| {
                if selection.loading {
                    return widgets::status(ui, true, "Loading seasons…");
                }
                if let Some(error) = &selection.error {
                    retry = padded_notice(ui, error);
                    return;
                }
                if selection.seasons.is_empty() {
                    return widgets::status(ui, false, "No seasons found on the series page.");
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (season, ticked) in &mut selection.seasons {
                            let (rect, _) = widgets::row(ui, ROW_HEIGHT, Sense::hover());
                            widgets::row_content(ui, rect, |ui| {
                                ui.checkbox(ticked, &season.label);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    widgets::site_link(
                                        ui,
                                        "Season page",
                                        season.url.as_str(),
                                        &mut self.pending_link,
                                    );
                                });
                            });
                        }
                    });
            });
        if retry {
            let series = selection.series.clone();
            selection.loading = true;
            self.send(Request::LoadSeasons(series));
        }
        if choose {
            self.choose_folder(ctx);
        }
        if start {
            self.start_download();
        }
    }

    fn choose_folder(&mut self, ctx: &egui::Context) {
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

    fn start_download(&mut self) {
        let (Some(selection), Some(folder)) = (&self.selection, self.folder.clone()) else {
            return;
        };
        let series = selection.series.clone();
        let seasons = selection
            .seasons
            .iter()
            .filter(|(_, ticked)| *ticked)
            .map(|(season, _)| season.clone())
            .collect();
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
