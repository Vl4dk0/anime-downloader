use crate::{
    download::{self, Outcome, Progress},
    site::{self, html::Entry, http::Http, Season, Series},
};
use anyhow::{Context, Result};
use eframe::egui;
use futures::{stream, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc,
};
use tokio::{
    runtime::Handle,
    sync::mpsc as async_mpsc,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use url::Url;

pub enum Request {
    LoadCatalog,
    LoadSeasons(Series),
    StartDownload {
        series: Series,
        seasons: Vec<Season>,
        folder: PathBuf,
    },
    RetryFailed,
    Cancel,
}

pub enum Event {
    CatalogLoaded {
        series: Vec<Series>,
        warnings: Vec<String>,
    },
    CatalogFailed(String),
    SeasonsLoaded {
        series_url: Url,
        seasons: Vec<Season>,
    },
    SeasonsFailed {
        series_url: Url,
        error: String,
    },
    JobQueued {
        total: usize,
    },
    FileStarted {
        id: usize,
        name: String,
        size: Option<u64>,
    },
    FileProgress {
        id: usize,
        bytes: u64,
    },
    FileDone {
        id: usize,
    },
    FileSkipped {
        id: usize,
    },
    FileFailed {
        id: usize,
        name: String,
        error: String,
    },
    // A recovered listing is replaced by its episodes, rather than counted as a file.
    FileRemoved {
        id: usize,
    },
    Finished,
}

#[derive(Clone)]
struct Sink {
    sender: mpsc::Sender<Event>,
    context: egui::Context,
}
impl Sink {
    fn send(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            self.context.request_repaint();
        }
    }
}

pub fn start(
    http: Http,
    runtime: &Handle,
    context: egui::Context,
) -> (async_mpsc::UnboundedSender<Request>, mpsc::Receiver<Event>) {
    let (requests, receiver) = async_mpsc::unbounded_channel();
    let (sender, events) = mpsc::channel();
    runtime.spawn(run(http, receiver, Sink { sender, context }));
    (requests, events)
}

async fn run(http: Http, mut requests: async_mpsc::UnboundedReceiver<Request>, sink: Sink) {
    let mut background = JoinSet::new();
    let mut catalog_cancel = CancellationToken::new();
    let mut seasons_cancel = CancellationToken::new();
    let mut job_cancel = CancellationToken::new();
    let mut job: Option<JoinHandle<Batch>> = None;
    let mut completed: Option<Batch> = None;
    loop {
        tokio::select! {
            result = async {
                match job.as_mut() {
                    Some(task) => task.await,
                    None => std::future::pending().await,
                }
            } => {
                job = None;
                match result {
                    Ok(batch) => completed = Some(batch),
                    Err(error) => sink.send(Event::FileFailed { id: usize::MAX, name: "Download job".into(), error: format!("background task failed: {error}") }),
                }
                sink.send(Event::Finished);
            }
            Some(result) = background.join_next(), if !background.is_empty() => {
                if let Err(error) = result { tracing::error!(%error, "background request failed"); }
            }
            request = requests.recv() => {
                let Some(request) = request else { break; };
                match request {
                    Request::LoadCatalog => {
                        catalog_cancel.cancel();
                        catalog_cancel = CancellationToken::new();
                        let cancel = catalog_cancel.clone();
                        let http = http.clone();
                        let sink = sink.clone();
                        background.spawn(async move {
                            let result = site::catalog::load(&http, &cancel).await;
                            if cancel.is_cancelled() { return; }
                            sink.send(match result {
                                Ok((series, warnings)) => Event::CatalogLoaded { series, warnings },
                                Err(error) => Event::CatalogFailed(format!("{error:#}")),
                            });
                        });
                    }
                    Request::LoadSeasons(series) => {
                        seasons_cancel.cancel();
                        seasons_cancel = CancellationToken::new();
                        let cancel = seasons_cancel.clone();
                        let http = http.clone();
                        let sink = sink.clone();
                        background.spawn(async move {
                            let result = site::series::seasons(&http, &series, &cancel).await;
                            if cancel.is_cancelled() { return; }
                            sink.send(match result {
                                Ok(seasons) => Event::SeasonsLoaded { series_url: series.url, seasons },
                                Err(error) => Event::SeasonsFailed { series_url: series.url, error: format!("{error:#}") },
                            });
                        });
                    }
                    Request::StartDownload { series, seasons, folder } if job.is_none() => {
                        let batch = Batch::new(series, seasons, folder);
                        job_cancel = CancellationToken::new();
                        job = Some(tokio::spawn(run_batch(http.clone(), batch, job_cancel.clone(), sink.clone())));
                        completed = None;
                    }
                    Request::RetryFailed if job.is_none() => {
                        if let Some(batch) = completed.take() {
                            job_cancel = CancellationToken::new();
                            job = Some(tokio::spawn(run_batch(http.clone(), batch, job_cancel.clone(), sink.clone())));
                        }
                    }
                    Request::Cancel => job_cancel.cancel(),
                    _ => {}
                }
            }
        }
    }
    catalog_cancel.cancel();
    seasons_cancel.cancel();
    job_cancel.cancel();
    background.abort_all();
    if let Some(job) = job {
        let _ = job.await;
    }
}

#[derive(Clone)]
enum WorkKind {
    Listing(Url),
    Episode(Entry),
}
#[derive(Clone)]
struct Work {
    id: usize,
    name: String,
    kind: WorkKind,
}
struct Batch {
    directory: Result<PathBuf>,
    pending: Vec<Work>,
    next_id: usize,
    total: usize,
    seen: HashSet<Url>,
    destinations: HashMap<String, Url>,
}
impl Batch {
    fn new(series: Series, seasons: Vec<Season>, folder: PathBuf) -> Self {
        let directory = download::safe_name(&series.title).map(|name| folder.join(name));
        let pending: Vec<_> = seasons
            .into_iter()
            .enumerate()
            .map(|(id, season)| Work {
                id,
                name: season.label,
                kind: WorkKind::Listing(season.url),
            })
            .collect();
        Self {
            directory,
            next_id: pending.len(),
            total: pending.len(),
            pending,
            seen: HashSet::new(),
            destinations: HashMap::new(),
        }
    }
    fn work(&mut self, name: String, kind: WorkKind) -> Work {
        let work = Work {
            id: self.next_id,
            name,
            kind,
        };
        self.next_id += 1;
        work
    }
}

async fn run_batch(http: Http, mut batch: Batch, cancel: CancellationToken, sink: Sink) -> Batch {
    let pending = std::mem::take(&mut batch.pending);
    batch.total = batch.total.saturating_sub(pending.len());
    for work in &pending {
        sink.send(Event::FileRemoved { id: work.id });
    }
    let directory_result = async {
        let directory = batch
            .directory
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error:#}"))?;
        tokio::fs::create_dir_all(directory)
            .await
            .with_context(|| format!("create {}", directory.display()))?;
        Ok::<_, anyhow::Error>(directory.clone())
    }
    .await;
    let directory = match directory_result {
        Ok(directory) => directory,
        Err(error) => {
            for work in pending {
                fail(&sink, &work, format!("{error:#}"));
                batch.pending.push(work);
                batch.total += 1;
            }
            sink.send(Event::JobQueued { total: batch.total });
            return batch;
        }
    };
    let (listings, mut episodes): (Vec<_>, Vec<_>) = pending
        .into_iter()
        .partition(|work| matches!(work.kind, WorkKind::Listing(_)));
    let mut discovery = stream::iter(listings.into_iter().map(|work| {
        let http = &http;
        let cancel = &cancel;
        async move {
            let result = match &work.kind {
                WorkKind::Listing(url) => site::series::episodes(http, url, cancel).await,
                WorkKind::Episode(_) => Err(anyhow::anyhow!("expected a listing task")),
            };
            (work, result)
        }
    }))
    .buffer_unordered(site::DISCOVERY_CONCURRENCY);
    while let Some((work, result)) = discovery.next().await {
        match result {
            Ok(found) => {
                for episode in found.episodes {
                    if batch.seen.insert(episode.url.clone()) {
                        episodes.push(batch.work(episode.name.clone(), WorkKind::Episode(episode)));
                    }
                }
                for (url, error) in found.failed_pages {
                    let work = batch.work(format!("Listing: {url}"), WorkKind::Listing(url));
                    fail(&sink, &work, error);
                    batch.pending.push(work);
                    batch.total += 1;
                }
            }
            Err(error) => {
                fail(&sink, &work, format!("{error:#}"));
                batch.pending.push(work);
                batch.total += 1;
            }
        }
    }
    batch.total += episodes.len();
    sink.send(Event::JobQueued { total: batch.total });
    let mut resolved = stream::iter(episodes.into_iter().map(|work| {
        let http = &http;
        let cancel = &cancel;
        async move {
            let result = match &work.kind {
                WorkKind::Episode(entry) => site::series::resolve(http, &entry.url, cancel).await,
                WorkKind::Listing(_) => Err(anyhow::anyhow!("expected an episode task")),
            };
            (work, result)
        }
    }))
    .buffer_unordered(site::DISCOVERY_CONCURRENCY);
    let mut downloads = Vec::new();
    let mut scheduled = HashSet::new();
    while let Some((work, result)) = resolved.next().await {
        let result = result.and_then(|url| {
            let raw = url.path_segments().and_then(|mut parts| parts.next_back()).context("video URL has no filename")?;
            let name = download::safe_name(raw)?;
            let key = name.to_ascii_lowercase();
            let dest = directory.join(name);
            if let Some(previous) = batch.destinations.get(&key) {
                anyhow::ensure!(previous == &url, "different videos share filename {}; cannot store both in the flat output folder", dest.display());
            }
            anyhow::ensure!(scheduled.insert(key.clone()), "multiple episodes share output file {}; cannot download them concurrently", dest.display());
            batch.destinations.insert(key, url.clone());
            Ok((url, dest))
        });
        match result {
            Ok((url, dest)) => downloads.push((work, url, dest)),
            Err(error) => {
                fail(&sink, &work, format!("{error:#}"));
                batch.pending.push(work);
            }
        }
    }
    let mut transfers = stream::iter(downloads.into_iter().map(|(work, url, dest)| {
        let http = &http;
        let cancel = &cancel;
        let sink = &sink;
        async move {
            let name = dest
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| work.name.clone());
            let result =
                download::download(
                    &http.client,
                    &url,
                    &dest,
                    cancel,
                    |progress| match progress {
                        Progress::Started { size, bytes } => {
                            sink.send(Event::FileStarted {
                                id: work.id,
                                name: name.clone(),
                                size,
                            });
                            sink.send(Event::FileProgress { id: work.id, bytes });
                        }
                        Progress::Bytes(bytes) => {
                            sink.send(Event::FileProgress { id: work.id, bytes })
                        }
                    },
                )
                .await;
            (work, result)
        }
    }))
    .buffer_unordered(site::DOWNLOAD_CONCURRENCY);
    while let Some((work, result)) = transfers.next().await {
        match result {
            Ok(Outcome::Downloaded) => sink.send(Event::FileDone { id: work.id }),
            Ok(Outcome::Skipped) => sink.send(Event::FileSkipped { id: work.id }),
            Err(error) => {
                fail(&sink, &work, format!("{error:#}"));
                batch.pending.push(work);
            }
        }
    }
    batch
}
fn fail(sink: &Sink, work: &Work, error: String) {
    sink.send(Event::FileFailed {
        id: work.id,
        name: work.name.clone(),
        error,
    });
}
