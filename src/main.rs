use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use inquire::MultiSelect;
use once_cell::sync::Lazy;
use percent_encoding::percent_decode_str;
use reqwest::header::RANGE;
use reqwest::Client;
use reqwest::StatusCode;
use scraper::{Html, Selector};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use url::Url;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DISCOVERY_CONCURRENCY: usize = 24;
const DOWNLOAD_CONCURRENCY: usize = 4;
const RETRIES: usize = 5;

#[derive(Debug, Clone, Copy)]
struct Source {
    key: &'static str,
    root: &'static str,
    dir: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct SourceChoice {
    source: &'static Source,
}

impl fmt::Display for SourceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.source.key, self.source.dir)
    }
}

#[derive(Debug)]
struct Config {
    output: PathBuf,
    discovery_concurrency: usize,
    download_concurrency: usize,
}

#[derive(Debug)]
struct DownloadJob {
    episode_url: String,
    mp4_url: String,
    dest: PathBuf,
}

#[derive(Debug, Parser)]
#[command(name = "scraper")]
#[command(about = "Scrape anime from cartoonsarea.cc")]
struct Cli {
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    #[arg(long, default_value_t = DISCOVERY_CONCURRENCY)]
    discovery: usize,

    #[arg(long, default_value_t = DOWNLOAD_CONCURRENCY)]
    download: usize,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    List,
    Download {
        sources: Vec<String>,

        #[arg(long, conflicts_with_all = ["sources", "select"])]
        all: bool,

        #[arg(long, conflicts_with_all = ["sources", "all"])]
        select: bool,
    },
}

const SOURCES: &[Source] = &[
    Source {
        key: "naruto",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/N-Subbed-Series/Naruto-Subbed-Videos/",
        dir: "Naruto",
    },
    Source {
        key: "naruto-shippuden",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/N-Subbed-Series/Naruto-Shippuden-Subbed-Videos/",
        dir: "NarutoShippuden",
    },
    Source {
        key: "boruto",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/B-Subbed-Series/Boruto-Naruto-Next-Generation-Subbed-Videos/",
        dir: "Boruto",
    },
    Source {
        key: "death-note",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/D-Subbed-Series/Death-Note-Subbed-Videos/",
        dir: "DeathNote",
    },
    Source {
        key: "one-punch-man",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/O-Subbed-Series/One-Punch-Man-Subbed-Videos/",
        dir: "OnePunchMan",
    },
    Source {
        key: "demon-slayer",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/K-Subbed-Series/Kimetsu-No-Yaiba-Subbed-Videos/",
        dir: "DemonSlayer",
    },
    Source {
        key: "death-note-en",
        root: "https://eng.cartoonsarea.cc/English-Dubbed-Series/D-Dubbed-Series/Death-Note-Dubbed-Videos",
        dir: "DeathNoteEn",
    },
    Source {
        key: "food-wars",
        root: "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/F-Subbed-Series/Food-Wars-Subbed-Videos/",
        dir: "FoodWars",
    },
];

static CONTENT_LINK_SEL: Lazy<Selector> =
    Lazy::new(|| Selector::parse("div.none div.Singamdasam a[href]").unwrap());
static PAGINATION_LINK_SEL: Lazy<Selector> =
    Lazy::new(|| Selector::parse("ul.pagination a[href]").unwrap());
static EPISODE_FILE_LINK_SEL: Lazy<Selector> =
    Lazy::new(|| Selector::parse("div.Box div.Singamdasam table a[href]").unwrap());
static DOWNLOAD_LINK_SEL: Lazy<Selector> =
    Lazy::new(|| Selector::parse("a.download-btn[href]").unwrap());

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(UA)
        .gzip(true)
        .timeout(Duration::from_secs(60 * 30))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(Into::into)
}

async fn fetch_html(client: &Client, url: &str) -> Result<String> {
    let mut last_error = None;
    for attempt in 1..=RETRIES {
        match fetch_html_once(client, url).await {
            Ok(body) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                return Ok(body);
            }
            Err(e) => {
                if attempt == RETRIES {
                    last_error = Some(e);
                    break;
                }
                let delay = retry_delay(attempt);
                eprintln!(
                    "! HTML fetch failed for {url} (attempt {attempt}/{RETRIES}): {e:#}; retrying in {}s",
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("HTML fetch failed for {url}")))
}

async fn fetch_html_once(client: &Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("status for {url}"))?;
    let body = resp.text().await?;
    Ok(body)
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_secs(2_u64.pow((attempt - 1) as u32))
}

fn resolve(base: &str, href: &str) -> Result<String> {
    let base_url = Url::parse(base).with_context(|| format!("parse base {base}"))?;
    let joined = base_url
        .join(href)
        .with_context(|| format!("join {href} onto {base}"))?;
    Ok(joined.to_string())
}

fn sanitize(stem_or_name: &str) -> String {
    stem_or_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.' || *c == '_')
        .collect()
}

fn clean_filename(raw: &str) -> String {
    let decoded = percent_decode_str(raw).decode_utf8_lossy().to_string();
    let trimmed = decoded.trim();
    let with_dash = if let Some((first, rest)) = trimmed.split_once(' ') {
        if first.chars().all(|c| c.is_ascii_digit()) {
            format!("{first}-{}", rest.replace(' ', ""))
        } else {
            trimmed.replace(' ', "")
        }
    } else {
        trimmed.to_string()
    };
    sanitize(&with_dash)
}

async fn sanitize_existing(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("? read_dir {}: {e:#}", dir.display());
                continue;
            }
        };
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let cleaned = sanitize(name);
            if cleaned.is_empty() || cleaned == name {
                continue;
            }
            let new_path = path.with_file_name(&cleaned);
            if new_path.exists() {
                eprintln!(
                    "? rename target exists, skipping: {} -> {}",
                    path.display(),
                    new_path.display()
                );
                continue;
            }
            println!("~ {} -> {}", path.display(), new_path.display());
            if let Err(e) = tokio::fs::rename(&path, &new_path).await {
                eprintln!("! rename failed {}: {e:#}", path.display());
            }
        }
    }
    Ok(())
}

async fn flatten_existing_season_dirs(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    let mut rd = tokio::fs::read_dir(root)
        .await
        .with_context(|| format!("read_dir {}", root.display()))?;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        let ft = entry.file_type().await?;
        if !ft.is_dir() {
            continue;
        }
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !dir_name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let mut season_rd = tokio::fs::read_dir(&path)
            .await
            .with_context(|| format!("read_dir {}", path.display()))?;
        while let Some(file_entry) = season_rd.next_entry().await? {
            let file_path = file_entry.path();
            let file_type = file_entry.file_type().await?;
            if !file_type.is_file() {
                continue;
            }
            let target = root.join(file_entry.file_name());
            if target.exists() {
                eprintln!(
                    "? flatten target exists, skipping: {} -> {}",
                    file_path.display(),
                    target.display()
                );
                continue;
            }
            println!("~ {} -> {}", file_path.display(), target.display());
            if let Err(e) = tokio::fs::rename(&file_path, &target).await {
                eprintln!("! flatten failed {}: {e:#}", file_path.display());
            }
        }
    }

    Ok(())
}

fn collect_links_by_selector(html: &str, base: &str, selector: &Selector) -> Vec<String> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for el in doc.select(selector) {
        if let Some(href) = el.value().attr("href") {
            match resolve(base, href) {
                Ok(abs) => {
                    if seen.insert(abs.clone()) {
                        out.push(abs);
                    }
                }
                Err(e) => eprintln!("? skip href {href}: {e:#}"),
            }
        }
    }
    out
}

fn collect_first_link_by_selector(html: &str, base: &str, selector: &Selector) -> Option<String> {
    collect_links_by_selector(html, base, selector)
        .into_iter()
        .next()
}

fn collect_content_links(html: &str, base: &str) -> Vec<String> {
    collect_links_by_selector(html, base, &CONTENT_LINK_SEL)
}

fn collect_pagination_links(html: &str, base: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for el in doc.select(&PAGINATION_LINK_SEL) {
        let class = el.value().attr("class").unwrap_or("");
        if class.split_whitespace().any(|name| name == "active") {
            continue;
        }
        if let Some(href) = el.value().attr("href") {
            match resolve(base, href) {
                Ok(abs) => {
                    if seen.insert(abs.clone()) {
                        out.push(abs);
                    }
                }
                Err(e) => eprintln!("? skip href {href}: {e:#}"),
            }
        }
    }
    out
}

async fn download(client: &Client, url: &str, dest: &Path) -> Result<()> {
    if let Ok(meta) = tokio::fs::metadata(dest).await {
        if meta.len() > 0 {
            println!("= skip {}", dest.display());
            return Ok(());
        }
    }

    let mut last_error = None;
    for attempt in 1..=RETRIES {
        match download_once(client, url, dest).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt == RETRIES {
                    last_error = Some(e);
                    break;
                }
                let delay = retry_delay(attempt);
                eprintln!(
                    "! download failed for {} (attempt {attempt}/{RETRIES}): {e:#}; retrying in {}s",
                    dest.display(),
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("download failed for {}", dest.display())))
}

async fn download_once(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let part_path: PathBuf = {
        let mut p = dest.as_os_str().to_owned();
        p.push(".part");
        PathBuf::from(p)
    };

    let existing_len = tokio::fs::metadata(&part_path)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);

    let mut request = client.get(url);
    if existing_len > 0 {
        request = request.header(RANGE, format!("bytes={existing_len}-"));
    }

    let resp = request.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();

    let (mut file, start_at) = if existing_len > 0 && status == StatusCode::PARTIAL_CONTENT {
        println!(
            "↻ resume {} from {:.1} MB",
            dest.display(),
            existing_len as f64 / 1_048_576.0
        );
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part_path)
            .await
            .with_context(|| format!("append {}", part_path.display()))?;
        (file, existing_len)
    } else {
        if existing_len > 0 && status == StatusCode::OK {
            eprintln!("! server ignored resume for {}, restarting", dest.display());
        }
        if !status.is_success() {
            resp.error_for_status_ref()
                .with_context(|| format!("status for {url}"))?;
        }
        let file = tokio::fs::File::create(&part_path)
            .await
            .with_context(|| format!("create {}", part_path.display()))?;
        (file, 0)
    };

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("stream chunk")?;
        downloaded += chunk.len() as u64;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part_path, dest).await?;
    let total = start_at + downloaded;
    println!(
        "↓ {} ({:.1} MB)",
        dest.display(),
        total as f64 / 1_048_576.0
    );
    Ok(())
}

async fn resolve_episode(
    client: &Client,
    episode_page_url: String,
    out_root: PathBuf,
) -> Result<DownloadJob> {
    let html = fetch_html(client, &episode_page_url).await?;
    let mp4_php_url =
        collect_first_link_by_selector(&html, &episode_page_url, &EPISODE_FILE_LINK_SEL)
            .ok_or_else(|| anyhow!("no episode file link on {episode_page_url}"))?;

    let html = fetch_html(client, &mp4_php_url).await?;
    let mp4_url = collect_first_link_by_selector(&html, &mp4_php_url, &DOWNLOAD_LINK_SEL)
        .ok_or_else(|| anyhow!("no download button on {mp4_php_url}"))?;

    let basename = mp4_url
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("file.mp4");
    let dest = out_root.join(clean_filename(basename));

    Ok(DownloadJob {
        episode_url: episode_page_url,
        mp4_url,
        dest,
    })
}

async fn collect_episodes_paginated(
    client: &Client,
    season_url: &str,
    discovery_concurrency: usize,
) -> Result<Vec<String>> {
    let first = fetch_html(client, season_url).await?;
    let mut episodes = Vec::new();
    let mut seen_episodes = std::collections::HashSet::new();
    for ep in collect_content_links(&first, season_url) {
        if seen_episodes.insert(ep.clone()) {
            episodes.push(ep);
        }
    }

    let mut seen_pages = std::collections::HashSet::new();
    seen_pages.insert(season_url.to_string());
    let page_urls = collect_pagination_links(&first, season_url)
        .into_iter()
        .filter(|page_url| seen_pages.insert(page_url.clone()))
        .collect::<Vec<_>>();

    let page_results = stream::iter(page_urls.into_iter().map(|page_url| async move {
        let result = fetch_html(client, &page_url).await;
        (page_url, result)
    }))
    .buffer_unordered(discovery_concurrency)
    .collect::<Vec<_>>()
    .await;

    for (page_url, result) in page_results {
        match result {
            Ok(html) => {
                for ep in collect_content_links(&html, &page_url) {
                    if seen_episodes.insert(ep.clone()) {
                        episodes.push(ep);
                    }
                }
            }
            Err(e) => eprintln!("! page fetch failed {page_url}: {e:#}"),
        }
    }
    Ok(episodes)
}

async fn scrape_series(
    client: &Client,
    series_url: &str,
    out_root: &Path,
    config: &Config,
) -> Result<()> {
    let label = out_root.display().to_string();
    println!("== {label}: {series_url}");
    tokio::fs::create_dir_all(out_root)
        .await
        .with_context(|| format!("create series directory {}", out_root.display()))?;

    let html = fetch_html(client, series_url).await?;
    let seasons = collect_content_links(&html, series_url);

    for season_url in seasons {
        let episodes =
            match collect_episodes_paginated(client, &season_url, config.discovery_concurrency)
                .await
            {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("! season fetch failed {season_url}: {e:#}");
                    continue;
                }
            };
        println!("[{label}] {season_url}: {} episodes", episodes.len());

        let client_ref = client;
        let out_root_ref = out_root.to_path_buf();
        let resolved = stream::iter(episodes.into_iter().map(|ep_url| {
            let dir = out_root_ref.clone();
            async move {
                let r = resolve_episode(client_ref, ep_url.clone(), dir).await;
                (ep_url, r)
            }
        }))
        .buffer_unordered(config.discovery_concurrency)
        .collect::<Vec<_>>()
        .await;

        let mut jobs = Vec::new();
        for (ep_url, result) in resolved {
            match result {
                Ok(job) => jobs.push(job),
                Err(e) => eprintln!("! {ep_url}: {e:#}"),
            }
        }

        println!(
            "[{label}] {season_url}: downloading {} files with concurrency {}",
            jobs.len(),
            config.download_concurrency
        );

        let download_results = stream::iter(jobs.into_iter().map(|job| async move {
            let episode_url = job.episode_url;
            let mp4_url = job.mp4_url;
            let dest = job.dest;
            let result = download(client_ref, &mp4_url, &dest).await;
            (episode_url, result)
        }))
        .buffer_unordered(config.download_concurrency)
        .collect::<Vec<_>>()
        .await;

        for (ep_url, result) in download_results {
            if let Err(e) = result {
                eprintln!("! {ep_url}: {e:#}");
            }
        }
    }
    Ok(())
}

fn find_source(key: &str) -> Result<&'static Source> {
    SOURCES
        .iter()
        .find(|s| s.key == key)
        .ok_or_else(|| anyhow!("unknown source: {key}"))
}

fn select_sources_interactive() -> Result<Vec<&'static Source>> {
    let choices = SOURCES
        .iter()
        .map(|source| SourceChoice { source })
        .collect::<Vec<_>>();

    let selected = MultiSelect::new("Select sources to download:", choices)
        .prompt()
        .context("select sources")?;

    if selected.is_empty() {
        return Err(anyhow!("no sources selected"));
    }

    Ok(selected.into_iter().map(|choice| choice.source).collect())
}

fn find_sources(keys: &[String]) -> Result<Vec<&'static Source>> {
    if keys.is_empty() {
        return Err(anyhow!(
            "no source selected; use `scraper list`, `scraper download --select`, or `scraper download --all`"
        ));
    }

    keys.iter().map(|key| find_source(key)).collect()
}

async fn scrape_selected_sources(
    client: &Client,
    config: &Config,
    selected: Vec<&'static Source>,
) -> Result<()> {
    for source in selected {
        let source_dir = config.output.join(source.dir);
        flatten_existing_season_dirs(&source_dir).await?;
        sanitize_existing(&source_dir).await?;

        scrape_series(client, source.root, &source_dir, config).await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config {
        output: cli.output,
        discovery_concurrency: cli.discovery,
        download_concurrency: cli.download,
    };
    let client = build_client()?;

    match cli.command {
        Command::List => {
            println!("Available sources:");
            for source in SOURCES {
                println!("> {}: {}", source.key, source.root);
            }
            return Ok(());
        }
        Command::Download {
            sources,
            all,
            select,
        } => {
            let selected = if all {
                SOURCES.iter().collect()
            } else if select {
                select_sources_interactive()?
            } else {
                find_sources(&sources)?
            };

            scrape_selected_sources(&client, &config, selected).await?;
        }
    }

    Ok(())
}
