use super::{
    theme::{self, Palette, SPACE_LG, SPACE_MD},
    widgets, App, FileState, LocalEvent, Screen,
};
use anime_downloader::engine::Request;
use eframe::egui::{self, vec2, Align, Align2, FontId, Layout, Rect, RichText, Sense, Ui};
use std::collections::BTreeMap;

const FILE_ROW_HEIGHT: f32 = 40.0;
const HEADER_ROW_HEIGHT: f32 = 30.0;
const SIZE_WIDTH: f32 = 140.0;
const SPEED_WIDTH: f32 = 90.0;
const BAR_HEIGHT: f32 = 8.0;
const MB: f64 = 1_048_576.0;

/// Column rects of the file table, shared by the header and every row so they
/// stay aligned.
struct Columns {
    name: Rect,
    bar: Rect,
    size: Rect,
    speed: Rect,
}

impl Columns {
    fn new(row: Rect) -> Self {
        let inner = row.shrink2(vec2(SPACE_LG, 0.0));
        let column = |left: f32, right: f32| {
            Rect::from_min_max(
                egui::pos2(left, inner.top()),
                egui::pos2(right, inner.bottom()),
            )
        };
        let speed = column(inner.right() - SPEED_WIDTH, inner.right());
        let size = column(speed.left() - SIZE_WIDTH, speed.left());
        let bar_width = (inner.width() * 0.22).clamp(80.0, 220.0);
        let bar = column(size.left() - SPACE_LG - bar_width, size.left() - SPACE_LG);
        let name = column(inner.left(), bar.left() - SPACE_LG);
        Self {
            name,
            bar,
            size,
            speed,
        }
    }
}

impl App {
    pub(super) fn progress_view(&mut self, ctx: &egui::Context) {
        let count = |wanted: fn(&FileState) -> bool| {
            self.job
                .files
                .values()
                .filter(|state| wanted(state))
                .count()
        };
        let done = count(|s| matches!(s, FileState::Done));
        let skipped = count(|s| matches!(s, FileState::Skipped));
        let failed = count(|s| matches!(s, FileState::Failed { .. }));
        let active = if self.job.finished {
            0
        } else {
            count(|s| matches!(s, FileState::Active(_)))
        };
        let completed = done + skipped + failed;
        let fraction = if self.job.total > 0 {
            completed as f32 / self.job.total as f32
        } else {
            0.0
        };
        let status = match (self.job.finished, self.job.cancelled) {
            (false, false) => "Downloading".to_owned(),
            (false, true) => "Cancelling…".to_owned(),
            (true, true) => "Cancelled. Partial files are kept for resuming.".to_owned(),
            (true, false) => {
                format!("Finished · Downloaded {done} · Skipped {skipped} · Failed {failed}")
            }
        };
        egui::TopBottomPanel::top("header")
            .frame(theme::header_frame(ctx))
            .show(ctx, |ui| {
                if let Some(selection) = &self.selection {
                    ui.horizontal(|ui| {
                        widgets::badge(ui, selection.series.kind);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&selection.series.title).heading().strong(),
                            )
                            .truncate(),
                        );
                    });
                    ui.horizontal(|ui| {
                        widgets::site_link(
                            ui,
                            "Series page",
                            selection.series.url.as_str(),
                            &mut self.pending_link,
                        );
                        let folder = format!("Saving to {}", self.job.folder.display());
                        ui.add(egui::Label::new(RichText::new(folder).weak()).truncate());
                    });
                }
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), BAR_HEIGHT), Sense::hover());
                widgets::progress_bar(ui, rect, fraction);
                ui.horizontal(|ui| {
                    if !self.job.finished {
                        ui.spinner();
                    }
                    ui.label(status);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(format!("{completed} / {}", self.job.total));
                    });
                });
            });
        let mut cancel = false;
        let mut retry = false;
        let mut open = false;
        let mut back = false;
        egui::TopBottomPanel::bottom("footer")
            .frame(theme::header_frame(ctx))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.job.finished {
                            cancel = widgets::secondary_button(ui, !self.job.cancelled, "Cancel")
                                .clicked();
                            return;
                        }
                        back = widgets::secondary_button(ui, true, "Back to catalog").clicked();
                        open = widgets::primary_button(ui, !self.opening_folder, "Open folder")
                            .clicked();
                        if failed > 0 || self.job.cancelled {
                            let label = if self.job.cancelled {
                                "Resume"
                            } else {
                                "Retry failed"
                            };
                            retry = widgets::secondary_button(ui, true, label).clicked();
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(theme::body_frame(ctx))
            .show(ctx, |ui| {
                if active == 0 && failed == 0 {
                    let text = if self.job.finished && self.job.cancelled {
                        "Partial downloads are kept for resuming."
                    } else if self.job.finished {
                        "No files were downloaded."
                    } else {
                        "Discovering episodes…"
                    };
                    return widgets::status(ui, !self.job.finished, text);
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if active > 0 {
                            table_header(ui);
                            for state in self.job.files.values() {
                                if let FileState::Active(file) = state {
                                    active_row(ui, file);
                                }
                            }
                        }
                        if failed > 0 {
                            failure_rows(ui, &self.job.files, failed);
                        }
                    });
            });
        if cancel {
            self.job.cancelled = true;
            self.send(Request::Cancel);
        }
        if retry {
            self.job.finished = false;
            self.job.cancelled = false;
            self.send(Request::RetryFailed);
        }
        if back {
            self.screen = Screen::Catalog;
        }
        if open {
            self.open_folder(ctx);
        }
    }

    fn open_folder(&mut self, ctx: &egui::Context) {
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
}

fn table_header(ui: &mut Ui) {
    let (rect, _) = widgets::row(ui, HEADER_ROW_HEIGHT, Sense::hover());
    let columns = Columns::new(rect);
    let muted = Palette::of(ui.visuals()).muted;
    let font = FontId::proportional(13.0);
    for (text, x, align) in [
        ("File", columns.name.left(), Align2::LEFT_CENTER),
        ("Progress", columns.bar.left(), Align2::LEFT_CENTER),
        ("Size", columns.size.right(), Align2::RIGHT_CENTER),
        ("Speed", columns.speed.right(), Align2::RIGHT_CENTER),
    ] {
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            align,
            text,
            font.clone(),
            muted,
        );
    }
}

fn active_row(ui: &mut Ui, file: &super::ActiveFile) {
    let (rect, _) = widgets::row(ui, FILE_ROW_HEIGHT, Sense::hover());
    let columns = Columns::new(rect);
    let fg = Palette::of(ui.visuals()).fg;
    let name = widgets::truncated(ui, &file.name, columns.name.width());
    let name_pos = egui::pos2(columns.name.left(), rect.center().y - name.size().y / 2.0);
    ui.painter().galley(name_pos, name, fg);
    let fraction = file
        .size
        .filter(|size| *size > 0)
        .map(|size| file.bytes as f32 / size as f32)
        .unwrap_or(0.0);
    let bar = Rect::from_center_size(columns.bar.center(), vec2(columns.bar.width(), BAR_HEIGHT));
    widgets::progress_bar(ui, bar, fraction);
    let total = file
        .size
        .map(|size| format!("{:.1}", size as f64 / MB))
        .unwrap_or_else(|| "?".into());
    let speed = if file.sampled_at.elapsed().as_secs() > 3 {
        0.0
    } else {
        file.speed
    };
    let mono = FontId::monospace(14.0);
    widgets::paint_text_right(
        ui,
        columns.size.right_center(),
        &format!("{:.1} / {total} MB", file.bytes as f64 / MB),
        mono.clone(),
        false,
    );
    widgets::paint_text_right(
        ui,
        columns.speed.right_center(),
        &format!("{:.1} MB/s", speed / MB),
        mono,
        true,
    );
}

/// Failures stay visible instead of collapsed: they need the owner's decision
/// (retry or check the site), so hiding them would hide the outcome.
fn failure_rows(ui: &mut Ui, files: &BTreeMap<usize, FileState>, count: usize) {
    ui.add_space(SPACE_MD);
    ui.horizontal(|ui| {
        ui.add_space(SPACE_LG);
        ui.label(RichText::new(format!("Failed ({count})")).strong());
    });
    ui.add_space(SPACE_MD);
    for state in files.values() {
        if let FileState::Failed { name, error } = state {
            ui.horizontal(|ui| {
                ui.add_space(SPACE_LG);
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(name).truncate());
                    ui.label(RichText::new(error).weak().small());
                });
            });
            ui.add_space(SPACE_MD);
        }
    }
}
