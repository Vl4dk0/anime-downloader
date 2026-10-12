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
    Cancelled,
}

pub async fn download(
    client: &Client,
    url: &Url,
    dest: &Path,
    cancel: &CancellationToken,
    report: impl Fn(Progress),
) -> Result<Outcome> {
    // Keep cancellation distinct from errors, including during filesystem work or backoff.
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Ok(Outcome::Cancelled),
        result = download_inner(client, url, dest, cancel, &report) => result,
    }
}

async fn download_inner(
    client: &Client,
    url: &Url,
    dest: &Path,
    cancel: &CancellationToken,
    report: &impl Fn(Progress),
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
        let result = download_once(client, url, dest, report).await;
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
    let range = response
        .headers()
        .get(CONTENT_RANGE)
        .map(|value| value.to_str())
        .transpose()
        .context("invalid Content-Range header")?;
    let size = expected_size(status, start, response.content_length(), range)
        .with_context(|| format!("validate response size for {url}"))?;
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
    file.flush()
        .await
        .with_context(|| format!("flush {}", part.display()))?;
    drop(file);
    validate_complete(bytes, size).with_context(|| format!("finish download for {url}"))?;
    tokio::fs::rename(&part, dest)
        .await
        .with_context(|| format!("finish {}", dest.display()))?;
    report(Progress::Bytes(bytes));
    Ok(())
}

// A range's length describes only this response; its total describes the whole video.
fn expected_size(
    status: StatusCode,
    start: u64,
    length: Option<u64>,
    range: Option<&str>,
) -> Result<Option<u64>> {
    let length_total = length
        .map(|length| start.checked_add(length).context("download size overflow"))
        .transpose()?;
    let Some(range) = range else {
        anyhow::ensure!(
            status != StatusCode::PARTIAL_CONTENT,
            "missing resume range"
        );
        return Ok(length_total);
    };
    let (bounds, total) = range
        .strip_prefix("bytes ")
        .and_then(|range| range.split_once('/'))
        .context("invalid Content-Range")?;
    let (first, last) = bounds.split_once('-').context("invalid range bounds")?;
    let first = first.parse::<u64>().context("invalid range start")?;
    let last = last.parse::<u64>().context("invalid range end")?;
    anyhow::ensure!(first == start && last >= first, "invalid resume range");
    let end = last.checked_add(1).context("range size overflow")?;
    anyhow::ensure!(
        length_total.is_none_or(|length_total| length_total == end),
        "Content-Length disagrees with Content-Range"
    );
    if total == "*" {
        return Ok(None);
    }
    let total = total.parse::<u64>().context("invalid range total")?;
    anyhow::ensure!(end <= total, "range exceeds video size");
    Ok(Some(total))
}

fn validate_complete(bytes: u64, size: Option<u64>) -> Result<()> {
    let size = size.context("unknown video size; partial download kept for resuming")?;
    anyhow::ensure!(
        bytes == size,
        "incomplete download: {bytes} of {size} bytes received"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_sizes_distinguish_ranges_from_complete_videos() {
        let partial = StatusCode::PARTIAL_CONTENT;
        assert_eq!(
            expected_size(StatusCode::OK, 0, Some(10), None).unwrap(),
            Some(10)
        );
        assert_eq!(expected_size(StatusCode::OK, 0, None, None).unwrap(), None);
        assert_eq!(
            expected_size(StatusCode::OK, 0, None, Some("bytes 0-9/10")).unwrap(),
            Some(10)
        );
        assert_eq!(
            expected_size(partial, 5, None, Some("bytes 5-9/10")).unwrap(),
            Some(10)
        );
        assert_eq!(
            expected_size(partial, 5, Some(3), Some("bytes 5-7/10")).unwrap(),
            Some(10)
        );
        assert_eq!(
            expected_size(partial, 5, None, Some("bytes 5-9/*")).unwrap(),
            None
        );
        assert_eq!(
            expected_size(partial, 5, Some(5), Some("bytes 5-9/*")).unwrap(),
            None
        );
        for range in [
            None,
            Some("bytes 4-9/10"),
            Some("bytes 5-10/10"),
            Some("bytes 5-4/10"),
            Some("bytes 5-9/no"),
        ] {
            assert!(expected_size(partial, 5, None, range).is_err());
        }
        assert!(expected_size(partial, 5, Some(3), Some("bytes 5-9/10")).is_err());
        assert!(expected_size(StatusCode::OK, 0, None, Some("invalid")).is_err());
        assert!(expected_size(partial, u64::MAX, Some(1), None).is_err());
    }

    #[test]
    fn only_known_complete_sizes_can_be_finalized() {
        assert!(validate_complete(10, Some(10)).is_ok());
        for (bytes, size) in [(5, Some(10)), (11, Some(10)), (10, None)] {
            assert!(validate_complete(bytes, size).is_err());
        }
        let size =
            expected_size(StatusCode::PARTIAL_CONTENT, 5, None, Some("bytes 5-9/10")).unwrap();
        assert!(validate_complete(10, size).is_ok());
        assert!(validate_complete(8, size).is_err());
        let size = expected_size(
            StatusCode::PARTIAL_CONTENT,
            5,
            Some(3),
            Some("bytes 5-7/10"),
        )
        .unwrap();
        assert!(validate_complete(8, size).is_err());
        let size =
            expected_size(StatusCode::PARTIAL_CONTENT, 5, Some(5), Some("bytes 5-9/*")).unwrap();
        assert!(validate_complete(10, size).is_err());
    }
}
