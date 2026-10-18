//! Link annotations: the clickable areas of a page and where they go.

use crate::doc::decode_text;
use crate::matrix as mx;
use crate::Pdf;
use anyhow::{anyhow, Result};
use lopdf::{Dictionary, ObjectId};
use std::collections::HashMap;

/// Limit that keeps a malformed or hostile file from running away.
const MAX_LINKS: usize = 10_000;

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of this document (zero-based).
    Page(usize),
    /// An address outside the document, as the file gives it (a web page, an email
    /// address, or anything else). Whether to open it is up to the caller.
    Uri(String),
}

/// One link on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// The clickable area in displayed-page points (y down), like
    /// [`crate::PageObject::bbox`].
    pub bbox: [f32; 4],
    pub target: LinkTarget,
}

impl Pdf {
    /// The links on a page (zero-based), in the file's order. Links that go to another
    /// file, run a program or go nowhere this document has are left out.
    pub fn page_links(&self, page_index: usize) -> Result<Vec<Link>> {
        let page = self.page_id_at(page_index)?;
        let (vd, _, _) = self.vis_down(page);
        let to_vis = mx::inv(vd).ok_or_else(|| anyhow!("degenerate page transform"))?;
        let Some(annots) = self
            .doc
            .get_dictionary(page)?
            .get(b"Annots")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
        else {
            return Ok(Vec::new());
        };
        let pages = self.page_index();
        let count = pages.len();
        let mut out = Vec::new();
        for a in annots.iter().take(MAX_LINKS) {
            let Some(a) = self
                .doc
                .dereference(a)
                .ok()
                .and_then(|(_, o)| o.as_dict().ok())
            else {
                continue;
            };
            if a.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Link") {
                continue;
            }
            let Some(rect) = a.get(b"Rect").ok().and_then(|o| self.rect(o)) else {
                continue;
            };
            if let Some(target) = self.link_target(a, &pages, page_index, count) {
                out.push(Link {
                    bbox: mx::transform_rect(to_vis, rect),
                    target,
                });
            }
        }
        Ok(out)
    }

    /// A link's `Dest`, or its action: `GoTo`, `URI`, or a named page action.
    fn link_target(
        &self,
        annot: &Dictionary,
        pages: &HashMap<ObjectId, usize>,
        here: usize,
        count: usize,
    ) -> Option<LinkTarget> {
        if let Ok(dest) = annot.get(b"Dest") {
            return self.dest_page(dest, pages, 0).map(LinkTarget::Page);
        }
        let (_, action) = self.doc.dereference(annot.get(b"A").ok()?).ok()?;
        let action = action.as_dict().ok()?;
        match action.get(b"S").ok()?.as_name().ok()? {
            b"GoTo" => self
                .dest_page(action.get(b"D").ok()?, pages, 0)
                .map(LinkTarget::Page),
            b"URI" => {
                let (_, uri) = self.doc.dereference(action.get(b"URI").ok()?).ok()?;
                let uri = decode_text(uri.as_str().ok()?);
                let uri = uri.trim();
                (!uri.is_empty()).then(|| LinkTarget::Uri(uri.to_string()))
            }
            b"Named" => {
                let last = count.checked_sub(1)?;
                let page = match action.get(b"N").ok()?.as_name().ok()? {
                    b"FirstPage" => 0,
                    b"LastPage" => last,
                    b"NextPage" => (here + 1).min(last),
                    b"PrevPage" => here.saturating_sub(1),
                    _ => return None,
                };
                Some(LinkTarget::Page(page))
            }
            _ => None,
        }
    }
}
