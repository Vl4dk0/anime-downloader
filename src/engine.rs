use crate::{
    download::{self, Outcome, Progress},
    site::{self, html::Entry, http::Http, Kind, Season, Series},
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
    // Per section, so the list can show whichever source answers first.
    SectionLoaded {
        kind: Kind,
        series: Vec<Series>,
    },
    SectionFailed {
        kind: Kind,
        reason: String,
    },
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
    // Retried runs discard old rows; recovered listings are replaced by their episodes.
    FileRemoved {
        id: usize,
    },
    Finished {
        cancelled: bool,
    },
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
    let mut job: Option<JoinHandle<(Batch, bool)>> = None;
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
                completed = finish_job(result, &sink);
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
                        for (kind, _, root) in site::catalog::SECTIONS {
                            background.spawn(load_section(http.clone(), kind, root, catalog_cancel.clone(), sink.clone()));
                        }
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
                    Request::Cancel => {
                        // Reclaim the batch before accepting another start or retry request.
                        if let Some(task) = job.take() {
                            job_cancel.cancel();
                            completed = finish_job(task.await, &sink);
                        }
                    }
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

async fn load_section(
    http: Http,
    kind: Kind,
    root: &'static str,
    cancel: CancellationToken,
    sink: Sink,
) {
    let result = site::catalog::load_section(&http, kind, root, &cancel).await;
    if cancel.is_cancelled() {
        return;
    }
    sink.send(match result {
        Ok(series) => Event::SectionLoaded { kind, series },
        Err(error) => {
            tracing::warn!(
                ?kind,
                error = format!("{error:#}"),
                "catalog section unavailable"
            );
            Event::SectionFailed {
                kind,
                reason: site::catalog::short_reason(&error),
            }
        }
    });
}

fn finish_job(
    result: std::result::Result<(Batch, bool), tokio::task::JoinError>,
    sink: &Sink,
) -> Option<Batch> {
    let (batch, cancelled) = match result {
        Ok((batch, cancelled)) => (Some(batch), cancelled),
        Err(error) => {
            sink.send(Event::FileFailed {
                id: usize::MAX,
                name: "Download job".into(),
                error: format!("background task failed: {error}"),
            });
            (None, false)
        }
    };
    sink.send(Event::Finished { cancelled });
    batch
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

async fn run_batch(
    http: Http,
    mut batch: Batch,
    cancel: CancellationToken,
    sink: Sink,
) -> (Batch, bool) {
    for id in 0..batch.next_id {
        sink.send(Event::FileRemoved { id });
    }
    sink.send(Event::JobQueued {
        total: batch.pending.len(),
    });
    // Pending owns every unfinished item, so dropping any stage preserves retry work.
    // Capture the outcome here; a late Cancel may reach an already completed task.
    let cancelled = tokio::select! {
        biased;
        _ = cancel.cancelled() => true,
        cancelled = process_batch(&http, &mut batch, &cancel, &sink) => cancelled,
    };
    (batch, cancelled)
}

async fn process_batch(
    http: &Http,
    batch: &mut Batch,
    cancel: &CancellationToken,
    sink: &Sink,
) -> bool {
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
            if cancel.is_cancelled() {
                return true;
            }
            for work in &batch.pending {
                fail(sink, work, format!("{error:#}"));
            }
            return false;
        }
    };
    let (listings, mut episodes): (Vec<_>, Vec<_>) = batch
        .pending
        .clone()
        .into_iter()
        .partition(|work| matches!(work.kind, WorkKind::Listing(_)));
    let mut discovery = stream::iter(listings.into_iter().map(|work| async move {
        let result = match &work.kind {
            WorkKind::Listing(url) => site::series::episodes(http, url, cancel).await,
            WorkKind::Episode(_) => Err(anyhow::anyhow!("expected a listing task")),
        };
        (work, result)
    }))
    .buffer_unordered(site::DISCOVERY_CONCURRENCY);
    let mut listing_failures = Vec::new();
    while let Some((work, result)) = discovery.next().await {
        if cancel.is_cancelled() {
            return true;
        }
        match result {
            Ok(found) => {
                batch.pending.retain(|pending| pending.id != work.id);
                for episode in found.episodes {
                    if batch.seen.insert(episode.url.clone()) {
                        let work = batch.work(episode.name.clone(), WorkKind::Episode(episode));
                        batch.pending.push(work.clone());
                        episodes.push(work);
                    }
                }
                for (url, error) in found.failed_pages {
                    let work = batch.work(format!("Listing: {url}"), WorkKind::Listing(url));
                    batch.pending.push(work.clone());
                    listing_failures.push((work, error));
                }
            }
            Err(error) => {
                listing_failures.push((work, format!("{error:#}")));
            }
        }
    }
    sink.send(Event::JobQueued {
        total: batch.pending.len(),
    });
    for (work, error) in listing_failures {
        fail(sink, &work, error);
    }
    let mut resolved = stream::iter(episodes.into_iter().map(|work| async move {
        let result = match &work.kind {
            WorkKind::Episode(entry) => site::series::resolve(http, &entry.url, cancel).await,
            WorkKind::Listing(_) => Err(anyhow::anyhow!("expected an episode task")),
        };
        (work, result)
    }))
    .buffer_unordered(site::DISCOVERY_CONCURRENCY);
    let mut downloads = Vec::new();
    let mut scheduled = HashSet::new();
    while let Some((work, result)) = resolved.next().await {
        if cancel.is_cancelled() {
            return true;
        }
        let result = result.and_then(|url| {
            let raw = url.path_segments().and_then(|mut parts| parts.next_back()).context("video URL has no filename")?;
            let name = download::safe_name(raw)?;
            let key = name.to_ascii_lowercase();
            let dest = directory.join(name);
            if let Some(previous) = batch.destinations.get(&key) {
                anyhow::ensure!(previous == &url, "different videos share filename {}; cannot store both in the flat output folder", dest.display());
            }
            // One pending transfer remains responsible for retries of a shared video.
            if !scheduled.insert(key.clone()) {
                return Ok(None);
            }
            batch.destinations.insert(key, url.clone());
            Ok(Some((url, dest)))
        });
        match result {
            Ok(Some((url, dest))) => downloads.push((work, url, dest)),
            Ok(None) => {
                sink.send(Event::FileSkipped { id: work.id });
                batch.pending.retain(|pending| pending.id != work.id);
            }
            Err(error) => {
                fail(sink, &work, format!("{error:#}"));
            }
        }
    }
    let mut transfers = stream::iter(downloads.into_iter().map(|(work, url, dest)| async move {
        let name = dest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| work.name.clone());
        let result = download::download(
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
                Progress::Bytes(bytes) => sink.send(Event::FileProgress { id: work.id, bytes }),
            },
        )
        .await;
        (work, result)
    }))
    .buffer_unordered(site::DOWNLOAD_CONCURRENCY);
    while let Some((work, result)) = transfers.next().await {
        if cancel.is_cancelled() {
            return true;
        }
        match result {
            Ok(outcome) => {
                match outcome {
                    Outcome::Downloaded => sink.send(Event::FileDone { id: work.id }),
                    Outcome::Skipped => sink.send(Event::FileSkipped { id: work.id }),
                    Outcome::Cancelled => return true,
                }
                batch.pending.retain(|pending| pending.id != work.id);
            }
            Err(error) => {
                fail(sink, &work, format!("{error:#}"));
            }
        }
    }
    false
}
fn fail(sink: &Sink, work: &Work, error: String) {
    sink.send(Event::FileFailed {
        id: work.id,
        name: work.name.clone(),
        error,
    });
}
