//! Embeds `assets/icon.ico` as the Windows executable's file icon so Explorer
//! and pinned taskbar entries show it; other platforms need no build step.

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile()?;
    }
    Ok(())
}
