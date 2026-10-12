use super::{
    html,
    http::{fetch_html, Http},
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

pub async fn load(client: &Http, cancel: &CancellationToken) -> Result<(Vec<Series>, Vec<String>)> {
    let results =
        futures::future::join_all(SECTIONS.into_iter().map(|(kind, label, root)| async move {
            (label, load_section(client, kind, root, cancel).await)
        }))
        .await;
    let mut series = Vec::new();
    let mut warnings = Vec::new();
    for (label, result) in results {
        match result {
            Ok(items) => series.extend(items),
            Err(error) => {
                // Empty reachable catalogs signal a changed contract, rather than an outage.
                if error.downcast_ref::<html::LayoutError>().is_some() {
                    return Err(error);
                }
                warnings.push(format!("{label} catalog unavailable: {error:#}"));
            }
        }
    }
    if series.is_empty() {
        bail!("catalog unavailable: {}", warnings.join("; "));
    }
    series.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.url.as_str().cmp(b.url.as_str()))
    });
    Ok((series, warnings))
}

async fn load_section(
    client: &Http,
    kind: Kind,
    root: &str,
    cancel: &CancellationToken,
) -> Result<Vec<Series>> {
    let root = Url::parse(root).context("parse section URL")?;
    let body = fetch_html(client, &root, cancel).await?;
    let letters = html::entries(&body, &root)?;
    let mut results = stream::iter(letters.into_iter().map(|letter| async move {
        let body = fetch_html(client, &letter.url, cancel).await?;
        html::entries(&body, &letter.url)
    }))
    .buffer_unordered(DISCOVERY_CONCURRENCY);
    let mut series = Vec::new();
    let mut seen = HashSet::new();
    while let Some(result) = results.next().await {
        for entry in result? {
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
        bail!("no series found on {root}; site layout may have changed");
    }
    Ok(series)
}
