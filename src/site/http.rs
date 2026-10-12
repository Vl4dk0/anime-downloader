use super::{DISCOVERY_CONCURRENCY, RETRIES};
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use url::Url;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct Http {
    pub client: Client,
    discovery: Arc<Semaphore>,
}

pub fn build_client() -> Result<Http> {
    let client = Client::builder()
        .user_agent(UA)
        .gzip(true)
        .timeout(Duration::from_secs(60 * 30))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .context("create HTTP client")?;
    Ok(Http {
        client,
        discovery: Arc::new(Semaphore::new(DISCOVERY_CONCURRENCY)),
    })
}

pub fn retry_delay(attempt: usize) -> Duration {
    Duration::from_secs(2_u64.pow((attempt.saturating_sub(1)) as u32))
}

pub async fn fetch_html(http: &Http, url: &Url, cancel: &CancellationToken) -> Result<String> {
    let mut last_error = None;
    for attempt in 1..=RETRIES {
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(anyhow!("cancelled")),
            result = fetch_html_once(http, url) => result,
        };
        match result {
            Ok(body) => {
                pause(Duration::from_millis(500), cancel).await?;
                return Ok(body);
            }
            Err(error) => {
                tracing::warn!(%url, attempt, %error, "HTML fetch failed");
                last_error = Some(error);
                if attempt < RETRIES {
                    pause(retry_delay(attempt), cancel).await?;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("HTML fetch failed for {url}")))
}

async fn fetch_html_once(http: &Http, url: &Url) -> Result<String> {
    let _permit = http
        .discovery
        .acquire()
        .await
        .context("acquire discovery slot")?;
    let response = http
        .client
        .get(url.clone())
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {} for {url}", status.as_u16());
    }
    response.text().await.with_context(|| format!("read {url}"))
}

pub async fn pause(duration: Duration, cancel: &CancellationToken) -> Result<()> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(anyhow!("cancelled")),
        _ = tokio::time::sleep(duration) => Ok(()),
    }
}
