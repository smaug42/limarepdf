// On Windows, embeds assets/icon.ico and the product name in limarepdf.exe, so Explorer,
// shortcuts and "Open with" show the icon and "LimarePDF" (Windows takes the name from
// FileDescription; left alone, winresource would fill it with the crate's description).
// The window and taskbar icon is set at run time in main.rs on every platform.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // A cross-build from Linux may lack a resource compiler (windres); build without the
        // embedded resources rather than fail.
        if let Err(e) = winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("FileDescription", "LimarePDF")
            .set("ProductName", "LimarePDF")
            .compile()
        {
            println!("cargo:warning=limarepdf.exe built without its icon and name: {e}");
        }
    }
}
