//! Viewer preferences: the layout, side panel and page a document asks to open with.

use crate::{Pdf, Result};
use anyhow::bail;
use lopdf::Object;

/// The page layout a document opens with (its `/PageLayout`), in MPE's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenLayout {
    /// One column of pages, scrolled (`OneColumn`).
    Continuous,
    /// One page at a time (`SinglePage`).
    Single,
    /// Two pages side by side, odd pages on the left (`TwoColumnLeft`, `TwoPageLeft`).
    Facing,
    /// Two pages side by side with the first page alone on the right (`TwoColumnRight`, `TwoPageRight`).
    FacingCover,
}

impl OpenLayout {
    pub const ALL: [OpenLayout; 4] = [
        OpenLayout::Continuous,
        OpenLayout::Facing,
        OpenLayout::FacingCover,
        OpenLayout::Single,
    ];

    /// The name used on the command line and in listings.
    pub fn name(self) -> &'static str {
        match self {
            OpenLayout::Continuous => "continuous",
            OpenLayout::Single => "single",
            OpenLayout::Facing => "facing",
            OpenLayout::FacingCover => "facing-cover",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == s)
    }

    fn pdf_name(self) -> &'static str {
        match self {
            OpenLayout::Continuous => "OneColumn",
            OpenLayout::Single => "SinglePage",
            OpenLayout::Facing => "TwoColumnLeft",
            OpenLayout::FacingCover => "TwoColumnRight",
        }
    }

    fn from_pdf_name(n: &[u8]) -> Option<Self> {
        Some(match n {
            b"OneColumn" => OpenLayout::Continuous,
            b"SinglePage" => OpenLayout::Single,
            b"TwoColumnLeft" | b"TwoPageLeft" => OpenLayout::Facing,
            b"TwoColumnRight" | b"TwoPageRight" => OpenLayout::FacingCover,
            _ => return None,
        })
    }
}

/// The side panel a document opens with (its `/PageMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenPanel {
    /// Page thumbnails (`UseThumbs`).
    Pages,
    /// The bookmarks (`UseOutlines`).
    Bookmarks,
}

impl OpenPanel {
    pub const ALL: [OpenPanel; 2] = [OpenPanel::Pages, OpenPanel::Bookmarks];

    /// The name used on the command line and in listings.
    pub fn name(self) -> &'static str {
        match self {
            OpenPanel::Pages => "pages",
            OpenPanel::Bookmarks => "bookmarks",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    fn pdf_name(self) -> &'static str {
        match self {
            OpenPanel::Pages => "UseThumbs",
            OpenPanel::Bookmarks => "UseOutlines",
        }
    }
}

/// What a document asks a viewer to open with. `None` means the document doesn't say (or says something MPE
/// doesn't use, such as full screen).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewPrefs {
    pub layout: Option<OpenLayout>,
    pub panel: Option<OpenPanel>,
    /// The zero-based page the document opens on.
    pub page: Option<usize>,
}

impl Pdf {
    /// The document's viewer preferences, from the catalog's `PageLayout`, `PageMode` and `OpenAction`.
    pub fn view_prefs(&self) -> ViewPrefs {
        let Ok(cat) = self.doc.catalog() else {
            return ViewPrefs::default();
        };
        let name = |key: &[u8]| -> Option<Vec<u8>> {
            let (_, o) = self.doc.dereference(cat.get(key).ok()?).ok()?;
            o.as_name().ok().map(<[u8]>::to_vec)
        };
        let layout = name(b"PageLayout").and_then(|n| OpenLayout::from_pdf_name(&n));
        let panel = name(b"PageMode").and_then(|n| match n.as_slice() {
            b"UseThumbs" => Some(OpenPanel::Pages),
            b"UseOutlines" => Some(OpenPanel::Bookmarks),
            _ => None,
        });
        ViewPrefs {
            layout,
            panel,
            page: self.open_page(),
        }
    }

    /// The page the `OpenAction` goes to, when it is a destination or a `GoTo` action.
    fn open_page(&self) -> Option<usize> {
        let cat = self.doc.catalog().ok()?;
        let (_, action) = self.doc.dereference(cat.get(b"OpenAction").ok()?).ok()?;
        let pages = self.page_index();
        match action {
            Object::Array(_) => self.dest_page(action, &pages, 0),
            Object::Dictionary(d) if d.get(b"S").and_then(Object::as_name).ok()? == b"GoTo" => {
                self.dest_page(d.get(b"D").ok()?, &pages, 0)
            }
            _ => None,
        }
    }

    /// Set the viewer preferences. Only what differs from [`Pdf::view_prefs`] is written, so a preference MPE
    /// reads the same way (such as `TwoPageLeft` for facing pages) is kept as it is. Clearing the page removes an
    /// `OpenAction` only when it goes to a page; other opening actions are left alone.
    pub fn set_view_prefs(&mut self, prefs: &ViewPrefs) -> Result<()> {
        if let Some(p) = prefs.page {
            let n = self.page_count();
            if p >= n {
                bail!("page {} does not exist (document has {n})", p + 1);
            }
        }
        let old = self.view_prefs();
        let page_id = prefs.page.map(|p| self.page_ids()[p]);
        let cat = self.doc.catalog_mut()?;
        if prefs.layout != old.layout {
            match prefs.layout {
                Some(l) => cat.set("PageLayout", Object::Name(l.pdf_name().into())),
                None => {
                    cat.remove(b"PageLayout");
                }
            }
        }
        if prefs.panel != old.panel {
            match prefs.panel {
                Some(p) => cat.set("PageMode", Object::Name(p.pdf_name().into())),
                None => {
                    cat.remove(b"PageMode");
                }
            }
        }
        if prefs.page != old.page {
            match page_id {
                Some(id) => cat.set(
                    "OpenAction",
                    vec![
                        Object::Reference(id),
                        Object::Name(b"XYZ".to_vec()),
                        Object::Null,
                        Object::Null,
                        Object::Null,
                    ],
                ),
                None => {
                    cat.remove(b"OpenAction");
                }
            }
        }
        Ok(())
    }
}
