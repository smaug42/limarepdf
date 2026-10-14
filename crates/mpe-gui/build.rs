// On Windows, embeds assets/icon.ico in mpe-gui.exe so Explorer and shortcuts show it.
// The window and taskbar icon is set at run time in main.rs on every platform.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // A cross-build from Linux may lack a resource compiler (windres); build without the
        // embedded icon rather than fail.
        if let Err(e) = winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
        {
            println!("cargo:warning=mpe-gui.exe built without its icon: {e}");
        }
    }
}
