use super::{
    html,
    http::{fetch_html, Http},
    Season, Series, DISCOVERY_CONCURRENCY,
};
use anyhow::Result;
use futures::{stream, StreamExt};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;
use url::Url;

pub fn season_number(url: &Url) -> Option<u32> {
    // Inspect only the last segment: episode paths also contain a season ancestor.
    let segment = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let lower = segment.to_ascii_lowercase();
    let (_, suffix) = lower.split_once("season-")?;
    let digits: String = suffix.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

pub fn parse_seasons(body: &str, series: &Series) -> Result<Vec<Season>> {
    let entries = html::entries(body, &series.url)?;
    if entries
        .iter()
        .any(|entry| season_number(&entry.url).is_some())
    {
        let mut seasons: Vec<_> = entries
            .into_iter()
            .map(|entry| Season {
                number: season_number(&entry.url),
                label: entry.name,
                url: entry.url,
            })
            .collect();
        seasons.sort_by_key(|season| (season.number.is_none(), season.number));
        Ok(seasons)
    } else {
        Ok(vec![Season {
            label: series.title.clone(),
            number: None,
            url: series.url.clone(),
        }])
    }
}

pub async fn seasons(
    http: &Http,
    series: &Series,
    cancel: &CancellationToken,
) -> Result<Vec<Season>> {
    parse_seasons(&fetch_html(http, &series.url, cancel).await?, series)
}

pub struct EpisodeDiscovery {
    pub episodes: Vec<html::Entry>,
    pub failed_pages: Vec<(Url, String)>,
}

pub async fn episodes(
    http: &Http,
    url: &Url,
    cancel: &CancellationToken,
) -> Result<EpisodeDiscovery> {
    let first = fetch_html(http, url, cancel).await?;
    let mut episodes = html::entries(&first, url)?;
    let mut seen: HashSet<_> = episodes.iter().map(|entry| entry.url.clone()).collect();
    let pages = html::pagination(&first, url);
    let mut pending = stream::iter(pages.into_iter().map(|page| async move {
        let result = async {
            let body = fetch_html(http, &page, cancel).await?;
            html::entries(&body, &page)
        }
        .await;
        (page, result)
    }))
    .buffer_unordered(DISCOVERY_CONCURRENCY);
    let mut failed_pages = Vec::new();
    while let Some((page, result)) = pending.next().await {
        match result {
            Ok(entries) => episodes.extend(
                entries
                    .into_iter()
                    .filter(|entry| seen.insert(entry.url.clone())),
            ),
            Err(error) => failed_pages.push((page, format!("{error:#}"))),
        }
    }
    episodes.sort_by(|a, b| a.url.as_str().cmp(b.url.as_str()));
    Ok(EpisodeDiscovery {
        episodes,
        failed_pages,
    })
}

pub async fn resolve(http: &Http, episode: &Url, cancel: &CancellationToken) -> Result<Url> {
    let body = fetch_html(http, episode, cancel).await?;
    // Some live seasons link straight to .mp4.php without an episode directory.
    let file = if episode.path().ends_with(".mp4.php") {
        return html::download_link(&body, episode);
    } else {
        html::episode_file(&body, episode)?
    };
    html::download_link(&fetch_html(http, &file, cancel).await?, &file)
}
