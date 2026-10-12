# Project instructions

Owner: Vladko. Reply in English unless asked otherwise. Keep communication
professional, concise, and specific. Ask for clarification when intent is unclear.
Find root causes; avoid temporary workarounds and unrelated improvements.

## Architecture

This is a Rust desktop application using `eframe`/`egui`, not a CLI. The approved
design is `docs/superpowers/specs/2026-10-12-gui-app-design.md`.

- `src/main.rs`: native window bootstrap and app-owned Tokio runtime.
- `src/site/html.rs`: pure HTML selectors and validated URL/name extraction.
- `src/site/http.rs`: shared HTTP client and retries.
- `src/site/catalog.rs`: discovery of the SUB/DUB catalogs.
- `src/site/series.rs`: seasons, episode pagination, and video resolution,
  including direct `.mp4.php` entries.
- `src/download.rs`: filenames and resumable `.part` downloads.
- `src/engine.rs`: background requests/events, cancellation, and batch jobs;
  listing failures and episode failures both support retry.
- `src/ui/`: app state, catalog, season selection, and download progress views.
- `tests/fixtures/`: trimmed real site HTML for deterministic parsing checks.
- `assets/`: app icon master plus derived `.icns`/`.ico`/256 px PNG (regenerate all from `icon.png` when it changes); `build.rs` embeds the `.ico` on Windows.
- `.github/workflows/`: Ubuntu/Windows checks and Windows/macOS release builds.

Keep network and filesystem work off the UI thread. Preserve partial downloads
on cancellation and tolerate individual episode failures. A failed DUB catalog
must not prevent SUB browsing. Handle series that list episodes without seasons.

## Code standards

- Split logic by responsibility. Do not consolidate the program into one file.
- No CLI flags, environment-variable settings, or test-only product code paths.
  Discovery/download concurrency and retry limits are constants.
- Validate URLs at the HTML boundary; log and skip invalid links.
- Return `anyhow::Result` with context from fallible functions. Surface useful
  errors to the UI, including an explanation when expected listing entries vanish.
- No `unwrap`/`expect` outside static selectors and tests.
- Doc comments explain intent and design reasons, not obvious function behavior.
- Keep tests few and meaningful (about 6–8); fixtures only, no network or GUI tests.
- Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and `cargo build --release` before completing implementation work.

Delegate focused tasks when useful; preserve other workers' edits.
Continue interrupted work without discarding useful changes. Delete scratch page
captures before committing; retain only trimmed fixtures. Commit in logical units
with `Co-Authored-By: Codex (gpt-6.1-sol) <noreply@openai.com>`.
Do not create a GitHub repository, push, or publish without the owner's explicit
instruction.
