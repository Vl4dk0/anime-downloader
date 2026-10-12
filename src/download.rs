use crate::site::{
    http::{pause, retry_delay},
    RETRIES,
};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use percent_encoding::percent_decode_str;
use reqwest::{
    header::{CONTENT_RANGE, RANGE},
    Client, StatusCode,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use url::Url;

pub fn clean_filename(raw: &str) -> String {
    let decoded = percent_decode_str(raw).decode_utf8_lossy();
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
    with_dash
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
        .collect()
}

// Reject special and Windows device names so site-controlled names stay portable.
pub fn safe_name(raw: &str) -> Result<String> {
    let name = clean_filename(raw);
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        bail!("unsafe or empty output name: {raw}");
    }
    Ok(name)
}

#[derive(Clone, Copy)]
pub enum Progress {
    Started { size: Option<u64>, bytes: u64 },
    Bytes(u64),
}
#[derive(Clone, Copy)]
pub enum Outcome {
    Downloaded,
    Skipped,
}

pub async fn download(
    client: &Client,
    url: &Url,
    dest: &Path,
    cancel: &CancellationToken,
    report: impl Fn(Progress),
) -> Result<Outcome> {
    match tokio::fs::metadata(dest).await {
        Ok(meta) if meta.is_file() && meta.len() > 0 => return Ok(Outcome::Skipped),
        Ok(meta) if !meta.is_file() => bail!("destination is not a file: {}", dest.display()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error).with_context(|| format!("inspect {}", dest.display()))
        }
        _ => {}
    }
    let mut last_error = None;
    for attempt in 1..=RETRIES {
        // Cancellation drops the transfer after any completed writes; the .part remains.
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
            result = download_once(client, url, dest, &report) => result,
        };
        match result {
            Ok(()) => return Ok(Outcome::Downloaded),
            Err(error) => {
                tracing::warn!(%url, attempt, %error, "download failed");
                last_error = Some(error);
                if attempt < RETRIES {
                    pause(retry_delay(attempt), cancel).await?;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("download failed for {}", dest.display())))
}

async fn download_once(
    client: &Client,
    url: &Url,
    dest: &Path,
    report: &impl Fn(Progress),
) -> Result<()> {
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let existing = match tokio::fs::metadata(&part).await {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error).with_context(|| format!("inspect {}", part.display())),
    };
    let mut request = client.get(url.clone());
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let response = request.send().await.with_context(|| format!("GET {url}"))?;
    let status = response.status();
    // An interrupted rename may leave a fully downloaded part file.
    if status == StatusCode::RANGE_NOT_SATISFIABLE && existing > 0 {
        let total = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes */"))
            .and_then(|v| v.parse::<u64>().ok());
        if total == Some(existing) {
            tokio::fs::rename(&part, dest)
                .await
                .with_context(|| format!("finish {}", dest.display()))?;
            return Ok(());
        }
    }
    response
        .error_for_status_ref()
        .with_context(|| format!("status for {url}"))?;
    let resumed = existing > 0 && status == StatusCode::PARTIAL_CONTENT;
    let start = if resumed { existing } else { 0 };
    if status == StatusCode::PARTIAL_CONTENT {
        let range_start = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes "))
            .and_then(|v| v.split_once('-'))
            .and_then(|(n, _)| n.parse::<u64>().ok());
        if range_start != Some(start) {
            bail!("invalid resume range for {url}");
        }
    }
    let size = response
        .content_length()
        .and_then(|len| len.checked_add(start));
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await
        .with_context(|| format!("open {}", part.display()))?;
    report(Progress::Started { size, bytes: start });
    let mut bytes = start;
    let mut last_report = Instant::now();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.with_context(|| format!("stream {url}"))?;
        file.write_all(&chunk)
            .await
            .with_context(|| format!("write {}", part.display()))?;
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .context("download size overflow")?;
        if last_report.elapsed() >= Duration::from_millis(100) {
            report(Progress::Bytes(bytes));
            last_report = Instant::now();
        }
    }
    if size.is_some_and(|size| bytes != size) {
        bail!("incomplete download for {url}: {bytes} bytes received");
    }
    file.flush()
        .await
        .with_context(|| format!("flush {}", part.display()))?;
    drop(file);
    tokio::fs::rename(&part, dest)
        .await
        .with_context(|| format!("finish {}", dest.display()))?;
    report(Progress::Bytes(bytes));
    Ok(())
}
