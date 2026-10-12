use super::{
    html,
    http::{fetch_html, Http, StatusError},
    Kind, Series, DISCOVERY_CONCURRENCY,
};
use anyhow::{bail, Context, Result};
use futures::{stream, StreamExt};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Public so the UI can link to the pages the catalog was crawled from.
pub const SECTIONS: [(Kind, &str, &str); 2] = [
    (
        Kind::Sub,
        "Japanese sub",
        "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/",
    ),
    (
        Kind::Dub,
        "English dub",
        "https://eng.cartoonsarea.cc/English-Dubbed-Series/",
    ),
];

/// The user is staring at an empty list while this runs, so a host that fails
/// twice is reported as down instead of being waited on through a long backoff.
const ATTEMPTS: usize = 2;

pub async fn load_section(
    client: &Http,
    kind: Kind,
    root: &str,
    cancel: &CancellationToken,
) -> Result<Vec<Series>> {
    let root = Url::parse(root).context("parse section URL")?;
    let body = fetch_html(client, &root, ATTEMPTS, cancel).await?;
    let letters = html::entries(&body, &root)?;
    let mut results = stream::iter(letters.into_iter().map(|letter| async move {
        let result = fetch_html(client, &letter.url, ATTEMPTS, cancel)
            .await
            .map(|body| html::listing(&body, &letter.url));
        (letter, result)
    }))
    .buffer_unordered(DISCOVERY_CONCURRENCY);
    let mut series = Vec::new();
    let mut seen = HashSet::new();
    let mut last_error = None;
    while let Some((letter, result)) = results.next().await {
        anyhow::ensure!(!cancel.is_cancelled(), "cancelled");
        let entries = match result {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(
                    ?kind,
                    letter = %letter.name,
                    url = %letter.url,
                    error = format!("{error:#}"),
                    "catalog letter unavailable"
                );
                last_error = Some(error);
                continue;
            }
        };
        for entry in entries {
            if seen.insert(entry.url.clone()) {
                series.push(Series {
                    title: entry.name,
                    url: entry.url,
                    kind,
                });
            }
        }
    }
    if series.is_empty() {
        if let Some(error) = last_error {
            return Err(error).context("no series collected from catalog letters");
        }
        bail!("no series found");
    }
    Ok(series)
}

/// The UI has room for a few words per unavailable section, not a URL-laden
/// error chain; the full error goes to the log instead.
pub fn short_reason(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(status) = cause.downcast_ref::<StatusError>() {
            return format!("HTTP {}", status.status);
        }
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            if error.is_timeout() {
                return "timed out".into();
            }
            if error.is_connect() {
                return "unreachable".into();
            }
        }
        if cause.downcast_ref::<html::LayoutError>().is_some() {
            return "unexpected page layout".into();
        }
    }
    error.to_string()
}

/// Sections arrive at different times; keeping one ordering means a late
/// section slots into the list instead of being appended after the first.
pub fn merge(catalog: &mut Vec<Series>, kind: Kind, series: Vec<Series>) {
    catalog.retain(|existing| existing.kind != kind);
    catalog.extend(series);
    catalog.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.url.as_str().cmp(b.url.as_str()))
    });
}
