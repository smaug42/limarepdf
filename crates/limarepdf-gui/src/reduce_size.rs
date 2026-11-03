//! Reduce file size's preview (ENH-19's stage 2): what reducing would do, worked out on a
//! thread of its own on a copy of the document, as Print's preview draws its sheets. Only
//! the newest choice is worked out: choices made while one is under way replace each other.

use eframe::egui;
use limarepdf_core::{Pdf, PictureQuality, ReduceOptions, ReducePreview, Reduced};
use std::sync::mpsc::{channel, Receiver, Sender};

/// What the thread is asked: the choice's number, the pages and the quality.
type Ask = (u64, Vec<usize>, PictureQuality);

enum Came {
    /// The size the document would be saved at now.
    Now(Result<usize, String>),
    /// What the choice numbered would do.
    Preview(u64, Result<ReducePreview, String>),
}

/// What the dialog shows of a choice worked out.
pub struct Shown {
    pub report: Reduced,
    /// The size the document would be saved at once reduced, in bytes.
    pub after: usize,
    /// The same part of the picture losing the most, before and after.
    pub parts: Option<(egui::TextureHandle, egui::TextureHandle)>,
}

/// The preview's thread and what it has sent.
pub struct Estimate {
    ask: Sender<Ask>,
    came: Receiver<Came>,
    /// The newest choice's number, and the choice.
    asked: Option<(u64, Vec<usize>, PictureQuality)>,
    pub now: Option<Result<usize, String>>,
    /// The newest choice worked out; `None` while it is under way.
    pub shown: Option<Result<Shown, String>>,
}

impl Estimate {
    /// Starts working on `pdf`, a copy of the document: first its size saved now. The thread
    /// ends once the estimate is dropped and it has finished what it was doing.
    pub fn spawn(ctx: &egui::Context, pdf: Pdf) -> Self {
        let (ask, asks) = channel::<Ask>();
        let (tx, came) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let now = crate::guard(|| pdf.saved_size()).map_err(|e| format!("{e:#}"));
            if tx.send(Came::Now(now)).is_err() {
                return;
            }
            ctx.request_repaint();
            while let Ok(mut next) = asks.recv() {
                // Only the newest choice.
                while let Ok(newer) = asks.try_recv() {
                    next = newer;
                }
                let (n, pages, quality) = next;
                let opts = ReduceOptions {
                    quality,
                    jpeg_quality: None,
                };
                let r = crate::guard(|| pdf.reduce_preview(&pages, &opts))
                    .map_err(|e| format!("{e:#}"));
                if tx.send(Came::Preview(n, r)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        Self {
            ask,
            came,
            asked: None,
            now: None,
            shown: None,
        }
    }

    /// Asks for `pages` at `quality`, unless that is what was asked last.
    pub fn ask(&mut self, pages: Vec<usize>, quality: PictureQuality) {
        if let Some((_, p, q)) = &self.asked {
            if *p == pages && *q == quality {
                return;
            }
        }
        let n = self.asked.as_ref().map_or(0, |a| a.0 + 1);
        let _ = self.ask.send((n, pages.clone(), quality));
        self.asked = Some((n, pages, quality));
        self.shown = None;
    }

    /// Takes what the thread has sent, making the pictures' textures.
    pub fn collect(&mut self, ctx: &egui::Context) {
        while let Ok(c) = self.came.try_recv() {
            match c {
                Came::Now(r) => self.now = Some(r),
                Came::Preview(n, r) if self.asked.as_ref().is_some_and(|a| a.0 == n) => {
                    self.shown = Some(r.map(|p| Shown {
                        parts: p.parts.map(|(a, b)| {
                            let tex = |name: &str, b: limarepdf_core::render::Bitmap| {
                                let img = egui::ColorImage::from_rgba_premultiplied(
                                    [b.width, b.height],
                                    &b.rgba,
                                );
                                ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
                            };
                            (tex("reduce-before", a), tex("reduce-after", b))
                        }),
                        report: p.report,
                        after: p.after,
                    }));
                }
                // An older choice's, replaced since.
                Came::Preview(..) => {}
            }
        }
    }

    /// Whether anything asked is still being worked out.
    #[cfg(test)]
    pub fn waiting(&self) -> bool {
        self.now.is_none() || (self.asked.is_some() && self.shown.is_none())
    }
}

/// `bytes` as people read file sizes: "840 KB", "12.4 MB" (decimal, as the CLI's).
pub fn size_text(bytes: usize) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    }
}

/// What reducing did, for the message after it: how many pictures, the size once saved,
/// and what was left and why.
pub fn reduced_text(r: &Reduced, saved: usize) -> String {
    let mut text = match r.pictures {
        0 => "No pictures were made smaller.".to_string(),
        1 => "1 picture was made smaller.".to_string(),
        n => format!("{n} pictures were made smaller."),
    };
    if r.pictures > 0 {
        text += &format!(" The file will be about {} when saved.", size_text(saved));
    }
    text += &kept_text(r);
    text
}

/// The pictures left as they are and why, each on a line of its own, or nothing.
pub fn kept_text(r: &Reduced) -> String {
    r.kept
        .iter()
        .map(|(why, n)| {
            let what = if *n == 1 { "picture" } else { "pictures" };
            format!("\n{n} {what} kept: {}.", why.describe())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use limarepdf_core::KeptWhy;

    #[test]
    fn sizes_and_reports_read_as_people_say_them() {
        assert_eq!(size_text(840_000), "840 KB");
        assert_eq!(size_text(400), "1 KB");
        assert_eq!(size_text(12_400_000), "12.4 MB");
        let mut r = Reduced {
            pictures: 3,
            ..Default::default()
        };
        r.kept.insert(KeptWhy::Kind, 1);
        r.kept.insert(KeptWhy::Resolution, 2);
        assert_eq!(
            reduced_text(&r, 3_100_000),
            "3 pictures were made smaller. The file will be about 3.1 MB when saved.\n2 \
             pictures kept: already at or under the resolution.\n1 picture kept: of a kind \
             that can't be made smaller."
        );
        let none = reduced_text(&Reduced::default(), 0);
        assert_eq!(none, "No pictures were made smaller.");
    }
}
