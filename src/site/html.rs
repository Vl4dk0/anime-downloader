use anyhow::Result;
use once_cell::sync::Lazy;
use scraper::{Html, Selector};
use std::{collections::HashSet, fmt};
use url::Url;

static CONTENT: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("div.directory-list a.directory-item[href]").expect("static content selector")
});
static NAME: Lazy<Selector> =
    Lazy::new(|| Selector::parse(".item-name").expect("static name selector"));
static PAGINATION: Lazy<Selector> =
    Lazy::new(|| Selector::parse("a.tab-btn[href]").expect("static pagination selector"));
static EPISODE_FILE: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("div.directory-list a.directory-item[href$=\".mp4.php\"]")
        .expect("static episode selector")
});
static DOWNLOAD: Lazy<Selector> =
    Lazy::new(|| Selector::parse("a.download-btn[href]").expect("static download selector"));

#[derive(Debug)]
pub struct LayoutError {
    url: Url,
    expected: &'static str,
}
impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "no {} found on {}; site layout may have changed",
            self.expected, self.url
        )
    }
}
impl std::error::Error for LayoutError {}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub url: Url,
}

fn links(
    html: &str,
    base: &Url,
    selector: &Selector,
    skip_active: bool,
    require_name: bool,
) -> Vec<Entry> {
    let doc = Html::parse_document(html);
    let mut seen = HashSet::new();
    doc.select(selector)
        .filter(|element| {
            !skip_active
                || !element
                    .value()
                    .attr("class")
                    .unwrap_or_default()
                    .split_whitespace()
                    .any(|class| class == "active")
        })
        .filter_map(|element| {
            let href = element.value().attr("href")?;
            let url = match base.join(href) {
                Ok(url) if matches!(url.scheme(), "http" | "https") => url,
                Ok(_) => {
                    tracing::warn!(%base, href, "skipping non-HTTP link");
                    return None;
                }
                Err(error) => {
                    tracing::warn!(%base, href, %error, "skipping invalid link");
                    return None;
                }
            };
            let name = element
                .select(&NAME)
                .next()
                .map(|name| name.text().collect::<String>())
                .unwrap_or_else(|| element.text().collect());
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            if name.is_empty() && require_name {
                tracing::warn!(%base, href, "skipping link without a name");
                return None;
            }
            if !seen.insert(url.clone()) {
                return None;
            }
            Some(Entry { name, url })
        })
        .collect()
}

pub fn entries(html: &str, base: &Url) -> Result<Vec<Entry>> {
    let entries = links(html, base, &CONTENT, false, true);
    if entries.is_empty() {
        return Err(LayoutError {
            url: base.clone(),
            expected: "entries",
        }
        .into());
    }
    Ok(entries)
}

pub fn pagination(html: &str, base: &Url) -> Vec<Url> {
    links(html, base, &PAGINATION, true, false)
        .into_iter()
        .map(|entry| entry.url)
        .filter(|url| url != base)
        .collect()
}

pub fn episode_file(html: &str, base: &Url) -> Result<Url> {
    first_link(html, base, &EPISODE_FILE, "episode file")
}

pub fn download_link(html: &str, base: &Url) -> Result<Url> {
    first_link(html, base, &DOWNLOAD, "download button")
}

fn first_link(html: &str, base: &Url, selector: &Selector, label: &'static str) -> Result<Url> {
    links(html, base, selector, false, false)
        .into_iter()
        .next()
        .map(|entry| entry.url)
        .ok_or_else(|| {
            LayoutError {
                url: base.clone(),
                expected: label,
            }
            .into()
        })
}
