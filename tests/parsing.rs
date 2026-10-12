use anime_downloader::{
    download::{clean_filename, safe_name},
    site::{
        html,
        series::{parse_seasons, season_number},
        Kind, Series,
    },
};
use url::Url;

const ROOT: &str = "https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/";
fn series() -> Series {
    Series {
        title: "Naruto Shippuden".into(),
        kind: Kind::Sub,
        url: Url::parse(&format!(
            "{ROOT}N-Subbed-Series/Naruto-Shippuden-Subbed-Videos/"
        ))
        .unwrap(),
    }
}
fn season_url() -> Url {
    series()
        .url
        .join("Naruto-Shippuden-Season-01-Subbed-Videos/")
        .unwrap()
}

#[test]
fn section_and_letter_preserve_names_and_resolve_protocol_relative_links() {
    let root = Url::parse(ROOT).unwrap();
    let letters = html::entries(include_str!("fixtures/section.html"), &root).unwrap();
    assert_eq!(letters.len(), 27);
    assert_eq!(letters[0].name, "A");
    assert_eq!(letters[26].name, "0...#");
    assert_eq!(letters[0].url.as_str(), format!("{ROOT}A-Subbed-Series/"));
    let letter = root.join("N-Subbed-Series/").unwrap();
    let entries = html::entries(include_str!("fixtures/letter.html"), &letter).unwrap();
    assert_eq!(entries.len(), 10);
    assert!(entries
        .iter()
        .any(|entry| entry.name == "Naruto Shippuden" && entry.url == series().url));
}

#[test]
fn seasons_are_classified_and_sorted_by_number() {
    let seasons = parse_seasons(include_str!("fixtures/series.html"), &series()).unwrap();
    assert_eq!(seasons.len(), 21);
    assert_eq!(seasons[0].number, Some(1));
    assert_eq!(seasons[20].number, Some(21));
    assert_eq!(seasons[0].url, season_url());
    let direct = html::entries(include_str!("fixtures/direct-season.html"), &series().url).unwrap();
    assert_eq!(direct.len(), 10);
    assert!(direct
        .iter()
        .all(|entry| entry.url.path().ends_with(".mp4.php")));
    let episode = season_url()
        .join("Naruto-Shippuden-Episode-01-Subbed-Video/")
        .unwrap();
    assert_eq!(season_number(&episode), None);
    assert_eq!(
        season_number(&Url::parse("https://example.org/X-Season-1-Videos/").unwrap()),
        Some(1)
    );
}

#[test]
fn real_seasonless_series_is_a_single_season_with_paginated_episodes() {
    let series = Series {
        title: "Gintama 2015".into(),
        kind: Kind::Sub,
        url: Url::parse(&format!(
            "{ROOT}G-Subbed-Series/Gintama-2015-Subbed-Videos/"
        ))
        .unwrap(),
    };
    let body = include_str!("fixtures/seasonless.html");
    let seasons = parse_seasons(body, &series).unwrap();
    assert_eq!(seasons.len(), 1);
    assert_eq!(seasons[0].number, None);
    assert_eq!(seasons[0].label, series.title);
    assert_eq!(seasons[0].url, series.url);
    let episodes = html::entries(body, &series.url).unwrap();
    assert_eq!(episodes.len(), 30);
    assert!(episodes
        .iter()
        .all(|entry| season_number(&entry.url).is_none()));
    assert!(!html::pagination(body, &series.url).is_empty());
}

#[test]
fn season_pagination_excludes_active_and_duplicate_next_links() {
    let body = include_str!("fixtures/season.html");
    let base = season_url();
    assert_eq!(html::entries(body, &base).unwrap().len(), 30);
    let pages = html::pagination(body, &base);
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0], base.join("?page=2").unwrap());
}

#[test]
fn episode_and_file_pages_resolve_the_real_video() {
    let episode = season_url()
        .join("Naruto-Shippuden-Episode-01-Subbed-Video/")
        .unwrap();
    let file = html::episode_file(include_str!("fixtures/episode.html"), &episode).unwrap();
    assert!(file.path().ends_with("01%20Homecoming.mp4.php"));
    let video = html::download_link(include_str!("fixtures/file.html"), &file).unwrap();
    assert!(video.path().contains("/USER-DATA/"));
    assert!(video.path().ends_with("01%20Homecoming.mp4"));
}

#[test]
fn malformed_links_are_skipped_and_duplicates_are_removed() {
    let base = Url::parse(ROOT).unwrap();
    let body = r#"<div class="directory-list">
      <a class="directory-item" href="http://[invalid"><span class="item-name">Bad</span></a>
      <a class="directory-item" href="javascript:alert(1)">Bad</a>
      <a class="directory-item" href="/good/"><span class="item-name"> </span></a>
      <a class="directory-item" href="/good/"><span class="item-name"> Good  Name </span></a>
      <a class="directory-item" href="/good/">Duplicate</a>
      <a class="directory-item" href="/empty/"><span class="item-name"> </span></a>
      </div>"#;
    let entries = html::entries(body, &base).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Good Name");
    assert_eq!(entries[0].url.as_str(), "https://www.cartoonsarea.cc/good/");
}

#[test]
fn missing_expected_entries_explain_a_changed_layout() {
    let base = Url::parse(ROOT).unwrap();
    for error in [
        html::entries("<p>empty</p>", &base).unwrap_err(),
        html::episode_file("<p>empty</p>", &base).unwrap_err(),
        html::download_link("<p>empty</p>", &base).unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(message.contains(ROOT));
        assert!(message.contains("site layout may have changed"));
    }
}

#[test]
fn filenames_keep_the_existing_flat_layout_and_stay_portable() {
    assert_eq!(clean_filename("01%20Homecoming.mp4"), "01-Homecoming.mp4");
    assert_eq!(
        clean_filename("  02 A: test? / title.mp4  "),
        "02-Atesttitle.mp4"
    );
    assert_eq!(clean_filename("Naruto Shippuden"), "NarutoShippuden");
    assert_eq!(clean_filename("A_B-C.mp4"), "A_B-C.mp4");
    assert_eq!(safe_name("01 Homecoming.mp4").unwrap(), "01-Homecoming.mp4");
    for name in ["", "..", ".", "CON.mp4", "LPT1.mp4", "///", "trailing."] {
        assert!(safe_name(name).is_err(), "{name}");
    }
}
