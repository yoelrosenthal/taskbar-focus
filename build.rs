//! Embeds Windows resources into the executable.
//!
//! This exists for one reason: an executable with **no version information at
//! all** is a genuine red flag to endpoint protection, and Rust produces one by
//! default. Filling it in costs nothing and gives both Windows and any security
//! analyst a straight answer about what this binary claims to be.
//!
//! It also attaches `taskbar-focus.manifest`, which declares `asInvoker` (this
//! program never asks for elevation) and enables themed common controls.

fn main() {
    println!("cargo:rerun-if-changed=taskbar-focus.manifest");
    println!("cargo:rerun-if-changed=focus-app-icons/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "taskbar-focus")
            .set(
                "FileDescription",
                "Focus/Pomodoro timer for the Windows notification area",
            )
            .set("CompanyName", "taskbar-focus contributors (open source)")
            .set(
                "LegalCopyright",
                "MIT licensed. Copyright (c) 2026 contributors",
            )
            .set("OriginalFilename", "taskbar-focus.exe")
            .set("InternalName", "taskbar-focus")
            .set("ProductVersion", version)
            .set("FileVersion", version)
            .set(
                "Comments",
                "Open source focus timer. Optionally switches Windows Do Not \
                 Disturb on during focus sessions and off during breaks; run \
                 with --explain for details. Requires no administrator rights. \
                 Source: https://github.com/yoelrosenthal/taskbar-focus",
            )
            .set_manifest_file("taskbar-focus.manifest")
            .set_icon("focus-app-icons/icon.ico");

        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed version resource: {e}");
        }
    }
}
