//! How the window's finished frames go to the screen (BUG-42). wgpu, which draws the window,
//! waits by default for the display to take each frame before the app goes on (vsync). On
//! Wayland that wait can last for ever: with NVIDIA's driver under KWin, a frame handed over
//! while the screen is off, or after a long idle gap, may not be taken, and the app stops
//! answering, the desktop with it, until the app is killed. On Wayland the compositor paces
//! drawing already (winit asks for a frame only after the compositor's frame callback says
//! one is wanted), so frames go over there without waiting, as eframe's OpenGL renderer does
//! too (<https://github.com/emilk/egui/issues/5136>). X11, Windows and macOS keep the wait.

use eframe::egui_wgpu::SurfaceConfig;
use eframe::wgpu::PresentMode;
#[cfg(all(unix, not(target_os = "macos")))]
use winit::raw_window_handle::RawDisplayHandle;

/// Sets how the window's frames go to the screen, for its window system. Cheap enough for
/// every frame: eframe passes the setting on to the renderer only when it changes, and before
/// the frame is drawn.
pub fn set(frame: &mut eframe::Frame) {
    if let Some(current) = frame.wgpu_surface_config() {
        frame.set_wgpu_surface_config(surface_config(on_wayland(frame), current));
    }
}

/// `current` with, on Wayland, frames handed over without waiting for the display: wgpu takes
/// the first the driver offers of immediate and mailbox, which never wait, and the waiting
/// queue (fifo), which every driver has, last. NVIDIA takes immediate and marks its frames as
/// fine to tear, which KWin allows only for a full-screen window it shows directly, with its
/// tearing setting on; mailbox alone (PB-66) would need the surface's modes, which eframe
/// doesn't give, as asking for a mode the surface lacks fails.
fn surface_config(wayland: bool, current: SurfaceConfig) -> SurfaceConfig {
    if wayland {
        SurfaceConfig {
            present_mode: PresentMode::AutoNoVsync,
            ..current
        }
    } else {
        current
    }
}

/// Whether the window is on Wayland, from the display the window system gave the app.
#[cfg(all(unix, not(target_os = "macos")))]
fn on_wayland(frame: &eframe::Frame) -> bool {
    use winit::raw_window_handle::HasDisplayHandle as _;
    is_wayland(frame.display_handle().ok().map(|d| d.as_raw()))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_wayland(display: Option<RawDisplayHandle>) -> bool {
    matches!(display, Some(RawDisplayHandle::Wayland(_)))
}

/// Windows and macOS have no Wayland.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn on_wayland(_frame: &eframe::Frame) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On Wayland frames go over without waiting for the display, which may never take them
    /// (BUG-42); the frame latency stays.
    #[test]
    fn wayland_frames_do_not_wait_for_the_display() {
        let usual = eframe::WgpuConfiguration::default().surface;
        let wayland = surface_config(true, usual);
        assert_eq!(wayland.present_mode, PresentMode::AutoNoVsync);
        assert_eq!(
            wayland.desired_maximum_frame_latency,
            usual.desired_maximum_frame_latency
        );
    }

    /// X11, Windows and macOS keep eframe's setting, which waits.
    #[test]
    fn elsewhere_frames_wait_as_before() {
        let usual = eframe::WgpuConfiguration::default().surface;
        assert_eq!(usual.present_mode, PresentMode::AutoVsync);
        assert_eq!(surface_config(false, usual), usual);
    }

    /// Only a Wayland display counts as Wayland; X11 under it, through XWayland, doesn't.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_display_says_wayland() {
        use std::ptr::NonNull;
        use winit::raw_window_handle::{WaylandDisplayHandle, XlibDisplayHandle};
        let wayland = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(NonNull::dangling()));
        let x11 = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
        assert!(is_wayland(Some(wayland)));
        assert!(!is_wayland(Some(x11)));
        assert!(!is_wayland(None));
    }
}
