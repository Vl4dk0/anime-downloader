pub mod catalog;
pub mod html;
pub mod http;
pub mod series;

use url::Url;

pub const DISCOVERY_CONCURRENCY: usize = 16;
pub const DOWNLOAD_CONCURRENCY: usize = 4;
pub const RETRIES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sub,
    Dub,
}

#[derive(Debug, Clone)]
pub struct Series {
    pub title: String,
    pub url: Url,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub struct Season {
    pub label: String,
    pub number: Option<u32>,
    pub url: Url,
}
