mod catalog_view;
mod progress_view;
mod series_view;
mod theme;
mod widgets;

use anime_downloader::{
    engine::{self, Event, Request},
    site::{catalog, http::Http, Season, Series},
};
use catalog_view::SectionState;
use eframe::egui;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::mpsc::UnboundedSender};

#[derive(PartialEq)]
enum Screen {
    Catalog,
    Series,
    Progress,
}
struct Selection {
    series: Series,
    seasons: Vec<(Season, bool)>,
    loading: bool,
    error: Option<String>,
}
struct ActiveFile {
    name: String,
    size: Option<u64>,
    bytes: u64,
    sampled_bytes: u64,
    sampled_at: Instant,
    speed: f64,
}
enum FileState {
    Active(ActiveFile),
    Done,
    Skipped,
    Failed { name: String, error: String },
}
#[derive(Default)]
struct Job {
    total: usize,
    files: BTreeMap<usize, FileState>,
    folder: PathBuf,
    finished: bool,
    cancelled: bool,
}
enum LocalEvent {
    DefaultFolder(Option<PathBuf>),
    Folder(Option<PathBuf>),
    OpenResult(Result<(), String>),
    LinkFailed(String),
}

pub struct App {
    runtime: Option<Runtime>,
    requests: UnboundedSender<Request>,
    events: mpsc::Receiver<Event>,
    local_sender: mpsc::Sender<LocalEvent>,
    local_events: mpsc::Receiver<LocalEvent>,
    screen: Screen,
    catalog: Vec<Series>,
    sections: catalog_view::Sections,
    search: String,
    selection: Option<Selection>,
    folder: Option<PathBuf>,
    choosing_folder: bool,
    opening_folder: bool,
    error: Option<String>,
    pending_link: Option<String>,
    job: Job,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, runtime: Runtime, http: Http) -> Self {
        theme::apply(&cc.egui_ctx);
        let (requests, events) = engine::start(http, runtime.handle(), cc.egui_ctx.clone());
        let (local_sender, local_events) = mpsc::channel();
        let sender = local_sender.clone();
        let context = cc.egui_ctx.clone();
        runtime.spawn_blocking(move || {
            let folder = dirs::download_dir().filter(|folder| folder.is_dir());
            let _ = sender.send(LocalEvent::DefaultFolder(folder));
            context.request_repaint();
        });
        let mut app = Self {
            runtime: Some(runtime),
            requests,
            events,
            local_sender,
            local_events,
            screen: Screen::Catalog,
            catalog: Vec::new(),
            sections: catalog_view::Sections::loading(),
            search: String::new(),
            selection: None,
            folder: None,
            choosing_folder: false,
            opening_folder: false,
            error: None,
            pending_link: None,
            job: Job::default(),
        };
        app.send(Request::LoadCatalog);
        app
    }
    fn send(&mut self, request: Request) {
        if self.requests.send(request).is_err() {
            self.error = Some("Background engine stopped. Close and reopen the app.".into());
        }
    }
    fn select_series(&mut self, series: Series) {
        self.screen = Screen::Series;
        self.selection = Some(Selection {
            series: series.clone(),
            seasons: Vec::new(),
            loading: true,
            error: None,
        });
        self.send(Request::LoadSeasons(series));
    }
    fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::SectionLoaded { kind, series } => {
                    self.sections.set(kind, SectionState::Loaded);
                    catalog::merge(&mut self.catalog, kind, series);
                }
                Event::SectionFailed { kind, reason } => {
                    self.sections.set(kind, SectionState::Unavailable(reason));
                }
                Event::SeasonsLoaded {
                    series_url,
                    seasons,
                } => {
                    if let Some(selection) = &mut self.selection {
                        if selection.series.url == series_url {
                            selection.seasons = seasons.into_iter().map(|s| (s, true)).collect();
                            selection.loading = false;
                            selection.error = None;
                        }
                    }
                }
                Event::SeasonsFailed { series_url, error } => {
                    if let Some(selection) = &mut self.selection {
                        if selection.series.url == series_url {
                            selection.loading = false;
                            selection.error = Some(error);
                        }
                    }
                }
                Event::JobQueued { total } => self.job.total = total,
                Event::FileStarted { id, name, size } => {
                    self.job.files.insert(
                        id,
                        FileState::Active(ActiveFile {
                            name,
                            size,
                            bytes: 0,
                            sampled_bytes: 0,
                            sampled_at: Instant::now(),
                            speed: 0.0,
                        }),
                    );
                }
                Event::FileProgress { id, bytes } => {
                    if let Some(FileState::Active(file)) = self.job.files.get_mut(&id) {
                        let elapsed = file.sampled_at.elapsed().as_secs_f64();
                        if file.bytes == 0 || bytes < file.sampled_bytes {
                            file.sampled_bytes = bytes;
                            file.sampled_at = Instant::now();
                            file.speed = 0.0;
                        } else if elapsed >= 0.3 {
                            file.speed = bytes.saturating_sub(file.sampled_bytes) as f64 / elapsed;
                            file.sampled_bytes = bytes;
                            file.sampled_at = Instant::now();
                        }
                        file.bytes = bytes;
                    }
                }
                Event::FileDone { id } => {
                    self.job.files.insert(id, FileState::Done);
                }
                Event::FileSkipped { id } => {
                    self.job.files.insert(id, FileState::Skipped);
                }
                Event::FileFailed { id, name, error } => {
                    self.job.files.insert(id, FileState::Failed { name, error });
                }
                Event::FileRemoved { id } => {
                    self.job.files.remove(&id);
                }
                Event::Finished => self.job.finished = true,
            }
        }
        while let Ok(event) = self.local_events.try_recv() {
            match event {
                LocalEvent::DefaultFolder(folder) => {
                    if self.folder.is_none() {
                        self.folder = folder;
                    }
                }
                LocalEvent::Folder(folder) => {
                    self.choosing_folder = false;
                    if folder.is_some() {
                        self.folder = folder;
                    }
                }
                LocalEvent::LinkFailed(error) => self.error = Some(error),
                LocalEvent::OpenResult(result) => {
                    self.opening_folder = false;
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
                }
            }
        }
    }
}
impl App {
    /// Opens in the system browser off the UI thread; egui's own opener is not
    /// compiled in with the features this app uses.
    fn open_link(&self, ctx: &egui::Context, url: String) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let sender = self.local_sender.clone();
        let context = ctx.clone();
        runtime.spawn_blocking(move || {
            if let Err(error) = opener::open(&url) {
                let _ = sender.send(LocalEvent::LinkFailed(format!("Open {url}: {error}")));
                context.request_repaint();
            }
        });
    }

    fn error_banner(&mut self, ctx: &egui::Context) {
        let Some(error) = self.error.clone() else {
            return;
        };
        let mut dismissed = false;
        egui::TopBottomPanel::top("error")
            .frame(theme::header_frame(ctx))
            .show(ctx, |ui| {
                dismissed = widgets::notice(ui, "Error", &error, Some("Dismiss"));
            });
        if dismissed {
            self.error = None;
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.error_banner(ctx);
        match self.screen {
            Screen::Catalog => self.catalog_view(ctx),
            Screen::Series => self.series_view(ctx),
            Screen::Progress => self.progress_view(ctx),
        }
        if let Some(url) = self.pending_link.take() {
            self.open_link(ctx, url);
        }
        if self.screen == Screen::Progress && !self.job.finished {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Cancel);
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}
