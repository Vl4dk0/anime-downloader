# Anime Downloader GUI — Design Spec

Date: 2026-10-12
Status: approved for implementation

## Goal

Turn the existing CLI scraper into a small desktop app that a non-technical friend
can download as a single file (Windows `.exe`, macOS `.app`), double-click, pick
any anime available on cartoonsarea.cc, tick the seasons they want, choose a
folder, and download.

Success criteria:

- Double-clicking the Windows `.exe` opens a window (no console window), with no
  installer and no runtime prerequisites.
- The catalog lists every series on the site, not a hard-coded subset.
- Downloads resume after interruption and survive transient network failures, as
  today.
- A tagged GitHub release produces downloadable Windows and macOS builds.

## Decisions (agreed with the owner)

| Topic | Decision |
|---|---|
| UI | Native GUI window, `eframe`/`egui`; folder picker via `rfd` |
| Catalog | Both sections, labelled `[SUB]` / `[DUB]` |
| Selection granularity | Series → seasons (checkboxes, all ticked by default) |
| CLI | Removed. GUI only. No `clap`, no hard-coded `SOURCES` list |
| Distribution | GitHub Actions on `v*` tags → GitHub Release; public repo created via `gh` |
| Output layout | `<chosen folder>/<SeriesDirName>/NN-Title.mp4` (flat, as today) |

Removed as obsolete: `sanitize_existing`, `flatten_existing_season_dirs` (one-off
migrations of the owner's old downloads), the clap `Cli`/`Command` types,
`inquire`.

## Site structure (verified 2026-10-12)

All listing pages share one markup: entries are
`div.directory-list a.directory-item[href]`, the visible name is in
`.item-name` inside the anchor. Hrefs are protocol-relative (`//www...`) and must
be resolved against the page URL.

| Level | URL example | Notes |
|---|---|---|
| Section root (SUB) | `https://www.cartoonsarea.cc/Japanese-Dubbed-Videos/` | 27 letter entries (`A-Subbed-Series/` … plus a `#`/digits entry) |
| Section root (DUB) | `https://eng.cartoonsarea.cc/English-Dubbed-Series/` | Currently returns HTTP 526 (Cloudflare origin down). `www.../English-Dubbed-Series/` redirects here. Assumed to share the same markup when up |
| Letter page | `.../N-Subbed-Series/` | All series of that letter on one page (no pagination seen) |
| Series page | `.../Naruto-Shippuden-Subbed-Videos/` | Season entries; names contain `Season-<n>` (`Season-01`, `Season-1`) |
| Season page | `.../Naruto-Shippuden-Season-01-Subbed-Videos/` | Episode entries; paginated with `a.tab-btn[href]` (`?page=N`), all page numbers are listed on page 1; the `active` one is the current page; "Next" duplicates a number link |
| Episode page | `.../Naruto-Shippuden-Episode-01-Subbed-Video/` | One `a.directory-item` whose href ends with `.mp4.php` |
| File page | `.../01 Homecoming.mp4.php` | `a.download-btn[href]` → `/USER-DATA/.../01 Homecoming.mp4` (direct file) |

Some series may have no season level (episodes directly on the series page).
The series page must therefore be classified: if its entries look like seasons
(`Season-\d+` in href), treat them as seasons; otherwise treat the series page
itself as a single season named after the series. The implementer must verify
this against at least one real non-seasoned series and handle it.

## Architecture

Split by responsibility; no module should hold the whole program.

```
src/
  main.rs            eframe bootstrap, #![windows_subsystem = "windows"]
  site/
    mod.rs
    html.rs          selectors, link + name extraction (pure, no I/O)
    http.rs          reqwest client, fetch_html with retry/backoff
    catalog.rs       crawl both section roots → Vec<Series>
    series.rs        seasons of a series, episodes of a season (pagination),
                     resolve episode → mp4 URL
  download.rs        resumable download (existing logic), reports progress
  engine.rs          tokio side: receives Requests, emits Events
  ui/
    mod.rs           App state + eframe::App impl, event polling
    catalog_view.rs  search box + filtered list
    series_view.rs   season checkboxes + folder picker + Download
    progress_view.rs overall/active progress, failures, retry, cancel
```

### Core types

```rust
enum Kind { Sub, Dub }
struct Series { title: String, url: Url, kind: Kind }
struct Season { label: String, number: Option<u32>, url: Url }
```

URLs are parsed into `url::Url` at the HTML boundary; anything that fails to
parse is skipped with a logged warning, never panics.

### UI ↔ engine

- The engine runs on a multi-thread tokio runtime owned by the app (started in
  `main`, kept alive for the app lifetime).
- UI → engine: `Request` over `tokio::sync::mpsc::UnboundedSender`:
  `LoadCatalog`, `LoadSeasons(Series)`, `StartDownload { series, seasons, folder }`,
  `RetryFailed`, `Cancel`.
- Engine → UI: `Event` over `std::sync::mpsc` (or crossbeam) channel; after
  sending, the engine calls `egui::Context::request_repaint()` so the UI wakes.
  Events: `CatalogLoaded { series, warnings }`, `CatalogFailed(String)`,
  `SeasonsLoaded { series_url, seasons }`, `SeasonsFailed(String)`,
  `JobQueued { total }`, `FileStarted { id, name, size }`,
  `FileProgress { id, bytes }`, `FileDone { id }`, `FileSkipped { id }`,
  `FileFailed { id, name, error }`, `Finished`.
- The UI drains events every frame; it never blocks on network I/O.
- Cancel uses a `tokio_util::sync::CancellationToken`; partial `.part` files are
  kept so the next run resumes.

### Concurrency (constants, not flags)

- Discovery (page fetches): 16 in flight.
- Downloads: 4 in flight.
- Retries: 5 attempts with exponential backoff (existing `retry_delay`).

### Download behaviour (unchanged from current code)

- Skip if destination exists and is non-empty.
- Write to `<dest>.part`; resume via `Range: bytes=<len>-`; on non-206 restart
  from zero; rename to final name when complete.
- Filenames via existing `clean_filename`.
- Series directory name: series title sanitized the same way as filenames
  (e.g. `Naruto Shippuden` → `NarutoShippuden`).

## Screens

1. **Catalog** — on start, `LoadCatalog` is sent automatically. Spinner while
   loading. Then a search box (case-insensitive substring filter) above a
   scrollable list of `[SUB] Naruto Shippuden` rows. If one section failed
   (e.g. DUB returns 526), show a non-blocking warning line ("English dub
   catalog unavailable: HTTP 526") and still list the other section. If both
   fail, or a reachable section yields zero series, show the error with a
   Retry button — never an empty list without explanation.
2. **Series** — clicking a series loads its seasons (spinner), shows them as
   checkboxes (all ticked), "Select all / none", a folder row with
   "Choose folder…" (`rfd::AsyncFileDialog` or the sync dialog spawned off the
   UI thread), Back, and Download (disabled until a folder is chosen and ≥1
   season ticked). Default folder: the user's Downloads directory (`dirs`
   crate) if it exists.
3. **Progress** — overall bar (files done / total) and one bar per active
   download (name, MB done/total, speed). A collapsible list of failures with
   "Retry failed". Cancel. When finished: summary (downloaded / skipped /
   failed), "Open folder" (`opener` or `open` crate), and Back to catalog.

## Error handling

- All fallible functions return `anyhow::Result` with context; errors reach the
  UI as human-readable strings. No `unwrap`/`expect` outside static selector
  construction and tests.
- Parsing a page that is expected to contain entries but yields none is an
  error mentioning the URL and "site layout may have changed".
- Network failures retry with backoff, then surface per item (one bad episode
  never aborts the batch).

## Testing (keep small: ~6–8 tests)

- Save trimmed real HTML fixtures under `tests/fixtures/` (section root, letter
  page, series page, season page with pagination, episode page, `.mp4.php`
  page).
- Unit tests for `site/html.rs` + `site/series.rs` parsing against fixtures
  (pure functions, no network).
- One test for `clean_filename`.
- No tests that hit the network. No GUI tests.

## Build & distribution

- `Cargo.toml`: package name `anime-downloader`, `[[bin]] name = "anime-downloader"`.
  Release profile: `lto = "thin"`, `strip = true`.
- `.github/workflows/release.yml`, triggered on `push: tags: ['v*']` (plus
  `workflow_dispatch`), matrix:
  - `windows-latest`, `x86_64-pc-windows-msvc` → `anime-downloader-windows-x64.exe`
  - `macos-latest`, `aarch64-apple-darwin` and `x86_64-apple-darwin` →
    `Anime Downloader.app` bundled (minimal `Info.plist`, either via
    `cargo-bundle` or a small script), zipped as
    `anime-downloader-macos-<arch>.zip`
  - Uploads assets to a GitHub Release for the tag (`softprops/action-gh-release`).
- `.github/workflows/ci.yml` on push/PR: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test` on ubuntu + windows.
  (Linux needs egui system deps installed in CI.)
- Builds are unsigned. README documents: Windows → "More info → Run anyway";
  macOS → right-click → Open.
- Add a short `README.md` (what it is, how to download a release, how to build).
- Update `AGENTS.md` to the new architecture (it is currently stale).

## Code standards (owner's rules — mandatory)

- No new CLI flags/env-var knobs; constants are fine.
- Doc comments explain *why/intent*, not what the code does.
- `cargo fmt` clean, `cargo clippy --all-targets -- -D warnings` clean.
- Keep tests few and meaningful (well under 100).
- Commit only your own work, in logical commits, with the trailer
  `Co-Authored-By` lines the orchestrator specifies.

## Out of scope

Episode-level selection, a persisted catalog cache, auto-update, code signing,
Linux release artifacts, settings UI for concurrency.
