# Anime Downloader

A small desktop app for cartoonsarea.cc. Search the SUB/DUB catalog, select
seasons, choose a folder, and download. Interrupted downloads resume from their
`.part` files. If the English dub site is unavailable, the SUB catalog still works.

## Download

Open this repository's **Releases** page and download:

- Windows: `anime-downloader-windows-x64.exe`; double-click to launch.
- Apple Silicon Mac: `anime-downloader-macos-arm64.zip`.
- Intel Mac: `anime-downloader-macos-x64.zip`.

On macOS, unzip and open **Anime Downloader.app** (macOS 11 or newer).
The builds are unsigned: on Windows, choose **More info → Run anyway** if
SmartScreen appears; on macOS, right-click the app and choose **Open**. If macOS
still blocks it, allow it in **System Settings → Privacy & Security → Open Anyway**.

Downloads are stored as `<chosen folder>/<SeriesName>/NN-Title.mp4`.
Use **Cancel** to stop, then select the same folder to resume later.

## Build

Install stable [Rust](https://rustup.rs/) and the platform's build tools
(Visual Studio C++ Build Tools on Windows; Xcode Command Line Tools on macOS).
On Ubuntu, also install:

```sh
sudo apt-get install libgtk-3-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

```sh
cargo build --release
cargo run --release
```

The executable is `target/release/anime-downloader` (`.exe` on Windows).
The app icon lives in `assets/` (`icon.png` master, `icon.icns` for the macOS
bundle, `icon.ico` embedded in the Windows exe by `build.rs`, `icon-256.png`
embedded for the window/Dock icon).
Tags beginning with `v` build Windows and both macOS releases in GitHub Actions.
A manual Release workflow run on a branch builds downloadable workflow artifacts;
running it on a `v` tag also publishes that tag's release.
