//! The desktop's accent colour, for View > Accent colour's Desktop (the user, 2026-10-27:
//! "It should pick up desktop theme colours if it can"): on Linux from the desktop portal's
//! `org.freedesktop.appearance` `accent-color` (GNOME 47 and KDE Plasma 6 on), on macOS the
//! system's accent, on Windows the colour it uses for its accents. `None` where the desktop
//! says none or can't be asked: LimarePDF's teal then ([`crate::theme::set_accent`]).

use eframe::egui::Color32;

/// The desktop's accent colour now. On Linux this asks the desktop portal over D-Bus, which
/// can take a moment: ask off the window's thread there ([`asks_slowly`]).
pub fn desktop() -> Option<Color32> {
    imp::read()
}

/// Whether [`desktop`] can take long enough to be asked on a thread of its own.
pub const fn asks_slowly() -> bool {
    cfg!(all(unix, not(target_os = "macos")))
}

/// A colour given as red, green and blue from 0 to 1; any other value means none is set.
#[cfg_attr(windows, allow(dead_code))]
fn from_unit(r: f64, g: f64, b: f64) -> Option<Color32> {
    let unit = |v: f64| (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8);
    Some(Color32::from_rgb(unit(r)?, unit(g)?, unit(b)?))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use eframe::egui::Color32;
    use std::time::Duration;
    use zbus::zvariant::OwnedValue;

    pub fn read() -> Option<Color32> {
        let conn = zbus::blocking::connection::Builder::session()
            .ok()?
            .method_timeout(Duration::from_secs(2))
            .build()
            .ok()?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "ReadOne",
                &("org.freedesktop.appearance", "accent-color"),
            )
            .ok()?;
        let value: OwnedValue = reply.body().deserialize().ok()?;
        let (r, g, b): (f64, f64, f64) = value.try_into().ok()?;
        super::from_unit(r, g, b)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use eframe::egui::Color32;
    use objc2_app_kit::{NSColor, NSColorSpace};

    pub fn read() -> Option<Color32> {
        let accent = NSColor::controlAccentColor();
        let c = accent.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        super::from_unit(c.redComponent(), c.greenComponent(), c.blueComponent())
    }
}

#[cfg(windows)]
mod imp {
    use eframe::egui::Color32;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    pub fn read() -> Option<Color32> {
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let key = wide("Software\\Microsoft\\Windows\\DWM");
        let name = wide("AccentColor");
        let mut abgr: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the key and value names are NUL-terminated wide strings that outlive the
        // call, and the data pointer and its size are those of `abgr`, a DWORD, the only
        // type RRF_RT_REG_DWORD lets through.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut abgr as *mut u32).cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        // 0xAABBGGRR.
        let [r, g, b, _] = abgr.to_le_bytes();
        Some(Color32::from_rgb(r, g, b))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn read() -> Option<eframe::egui::Color32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal's red, green and blue from 0 to 1; out of range is "not set".
    #[test]
    fn a_colour_out_of_range_is_none() {
        let breeze = from_unit(0.239216, 0.682353, 0.913725);
        assert_eq!(breeze, Some(Color32::from_rgb(61, 174, 233)));
        assert_eq!(from_unit(-1.0, 0.5, 0.5), None);
        assert_eq!(from_unit(0.5, 1.5, 0.5), None);
    }
}
