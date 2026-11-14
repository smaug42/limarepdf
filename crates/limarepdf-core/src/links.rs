//! Link annotations: the clickable areas of a page and where they go, and making and
//! changing them (ENH-17).
//!
//! New links go only to this document's pages, to web pages (`http`, `https`) and to email
//! addresses (`mailto`): [`web_link`] and [`email_link`] turn what was typed into the
//! address written, and every writer checks an address again before writing it. A link
//! of another kind that a file already has (another file, a program, a script) is read as
//! [`LinkTarget::Unfollowed`], so it can be shown, removed or pointed elsewhere; it is
//! never followed.

use crate::doc::decode_text;
use crate::form_ap::nums;
use crate::matrix as mx;
use crate::Pdf;
use anyhow::{anyhow, bail, Result};
use lopdf::{dictionary, Dictionary, Object, ObjectId, StringFormat};
use std::collections::{HashMap, HashSet};
use url::{Host, Url};

/// Limit that keeps a malformed or hostile file from running away.
const MAX_LINKS: usize = 10_000;
/// Annotation flag: printed.
const PRINT: i64 = 1 << 2;
/// The blue of a link's underline: Word's hyperlink colour (#0563C1).
pub(crate) const LINK_BLUE: [f32; 3] = [0.02, 0.39, 0.76];
/// The longest address a new link takes.
const MAX_ADDRESS: usize = 4096;

/// Why a typed address isn't taken, in the words the app shows.
const ONLY_WEB_AND_EMAIL: &str = "LimarePDF links to web pages and email addresses only.";
const NOT_WEB: &str = "That isn't a web address. Type one such as example.com.";
const NOT_EMAIL: &str = "That isn't an email address. Type one such as name@example.com.";
const TOO_LONG: &str = "That address is too long.";

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of this document (zero-based).
    Page(usize),
    /// A web page or an email address, as the file gives it (an address without a scheme
    /// too). Whether to open it is up to the caller.
    Uri(String),
    /// Something LimarePDF doesn't follow.
    Unfollowed(Unfollowed),
}

/// What a link LimarePDF doesn't follow does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unfollowed {
    /// Opens another file (`GoToR`, `GoToE`), by the name the link gives.
    File(String),
    /// Runs a program or opens a file with the system (`Launch`), by the name it gives.
    Program(String),
    /// Runs a script (`JavaScript`).
    Script,
    /// An address that isn't a web page or an email address (`file:`, `javascript:`, `tel:`
    /// and so on), as the file gives it.
    Address(String),
    /// Another action, by its PDF name (`Print`, `SubmitForm`, …).
    Action(String),
    /// Nowhere this document has: a page or name it doesn't have, or no target at all.
    Nowhere,
}

/// Which link, for changing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkId {
    /// The link's object in the file; stays the same while the document is open.
    Object((u32, u16)),
    /// A link written inside its page's list of annotations rather than as an object of
    /// its own: the page (zero-based) and its place in the list. Changing it makes it an
    /// object first, whose id the change returns.
    Inline { page: usize, at: usize },
}

/// One link on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: LinkId,
    /// The clickable area in displayed-page points (y down), like
    /// [`crate::PageObject::bbox`].
    pub bbox: [f32; 4],
    /// The areas that make it up, in the same points: one a line for a link over text
    /// (`/QuadPoints`), else just `bbox`.
    pub areas: Vec<[f32; 4]>,
    pub target: LinkTarget,
    /// It is shown underlined (`/BS /S /U` with a width), as LimarePDF's blue underline is.
    pub underlined: bool,
}

/// The address a new link to a web page is written with, from what was typed: `https://` is
/// put in front of an address without a scheme (`example.com`), and a name or path in
/// other alphabets is written in the ASCII form a link holds. An email address typed with
/// `mailto:` goes to [`email_link`]. Other schemes are refused, as are addresses that
/// aren't a web page's.
pub fn web_link(typed: &str) -> Result<String> {
    let t = typed.trim();
    if t.is_empty() {
        bail!("Type the web page's address.");
    }
    if t.len() > MAX_ADDRESS {
        bail!(TOO_LONG);
    }
    let full = match scheme_of(t).as_deref() {
        Some("http" | "https") => t.to_string(),
        Some("mailto") => return email_link(t),
        Some(_) => bail!(ONLY_WEB_AND_EMAIL),
        None => format!("https://{t}"),
    };
    let url = Url::parse(&full).map_err(|_| anyhow!(NOT_WEB))?;
    if url.host_str().is_none_or(str::is_empty) {
        bail!(NOT_WEB);
    }
    Ok(url.into())
}

/// The address a new link to an email address is written with (`mailto:`), from what was
/// typed, with or without `mailto:` and with a `?subject=` if given. A domain in other
/// alphabets is written in its ASCII form.
pub fn email_link(typed: &str) -> Result<String> {
    let t = typed.trim();
    let t = match t.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("mailto:") => &t[7..],
        _ => t,
    };
    if t.is_empty() {
        bail!("Type the email address.");
    }
    if t.len() > MAX_ADDRESS {
        bail!(TOO_LONG);
    }
    let (address, query) = match t.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (t, None),
    };
    let Some((name, domain)) = address.split_once('@') else {
        bail!(NOT_EMAIL);
    };
    let bad = |s: &str| s.is_empty() || s.chars().any(|c| c.is_whitespace() || c == '@');
    if bad(name) || bad(domain) {
        bail!(NOT_EMAIL);
    }
    let host = Host::parse(domain).map_err(|_| anyhow!(NOT_EMAIL))?;
    let query = query.map(|q| format!("?{q}")).unwrap_or_default();
    let url =
        Url::parse(&format!("mailto:{name}@{host}{query}")).map_err(|_| anyhow!(NOT_EMAIL))?;
    Ok(url.into())
}

/// The scheme an address starts with, in lower case; `None` when it has none. A host and
/// port (`example.com:8080`, `localhost:3000`) aren't taken for a scheme.
fn scheme_of(t: &str) -> Option<String> {
    let (s, rest) = t.split_once(':')?;
    let mut chars = s.chars();
    let named = chars.next()?.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    let port = rest.starts_with(|c: char| c.is_ascii_digit());
    (named && !port).then(|| s.to_ascii_lowercase())
}

/// Whether an address a file gives is one LimarePDF follows: a web page, an email address,
/// or an address without a scheme.
fn followed(uri: &str) -> bool {
    scheme_of(uri).is_none_or(|s| matches!(s.as_str(), "http" | "https" | "mailto"))
}

/// An address checked again before it is written: only the web and email addresses
/// [`web_link`] and [`email_link`] make.
fn checked(uri: &str) -> Result<String> {
    let url = Url::parse(uri.trim()).map_err(|_| anyhow!(ONLY_WEB_AND_EMAIL))?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some_and(|h| !h.is_empty()) => Ok(url.into()),
        "mailto" => email_link(url.as_str()),
        _ => bail!(ONLY_WEB_AND_EMAIL),
    }
}

impl Pdf {
    /// The links on a page (zero-based), in the file's order, those LimarePDF doesn't follow
    /// too. Links without an area are left out.
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
        for (at, a) in annots.iter().enumerate().take(MAX_LINKS) {
            let id = match a.as_reference() {
                Ok(id) => LinkId::Object(id),
                Err(_) => LinkId::Inline {
                    page: page_index,
                    at,
                },
            };
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
            let Some(rect) = self.annot_rect(a) else {
                continue;
            };
            let bbox = mx::transform_rect(to_vis, rect);
            let areas = self
                .numbers(a.get(b"QuadPoints").ok())
                .map(|q| quad_boxes(to_vis, &q))
                .filter(|b| !b.is_empty())
                .unwrap_or_else(|| vec![bbox]);
            out.push(Link {
                id,
                bbox,
                areas,
                target: self.link_target(a, &pages, page_index, count),
                underlined: self.link_underlined(a),
            });
        }
        Ok(out)
    }

    /// A link's `Dest`, or its action.
    fn link_target(
        &self,
        annot: &Dictionary,
        pages: &HashMap<ObjectId, usize>,
        here: usize,
        count: usize,
    ) -> LinkTarget {
        let page = |p: Option<usize>| {
            p.map_or(
                LinkTarget::Unfollowed(Unfollowed::Nowhere),
                LinkTarget::Page,
            )
        };
        if let Ok(dest) = annot.get(b"Dest") {
            return page(self.dest_page(dest, pages, 0));
        }
        let Some(action) = annot
            .get(b"A")
            .ok()
            .and_then(|o| self.doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
        else {
            return LinkTarget::Unfollowed(Unfollowed::Nowhere);
        };
        let Some(kind) = action.get(b"S").ok().and_then(|o| o.as_name().ok()) else {
            return LinkTarget::Unfollowed(Unfollowed::Nowhere);
        };
        let file = || self.file_named(action).unwrap_or_default();
        match kind {
            b"GoTo" => page(
                action
                    .get(b"D")
                    .ok()
                    .and_then(|d| self.dest_page(d, pages, 0)),
            ),
            b"URI" => {
                let uri = action
                    .get(b"URI")
                    .ok()
                    .and_then(|o| self.doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_str().ok())
                    .map(decode_text)
                    .unwrap_or_default();
                let uri = uri.trim();
                if uri.is_empty() {
                    LinkTarget::Unfollowed(Unfollowed::Nowhere)
                } else if followed(uri) {
                    LinkTarget::Uri(uri.to_string())
                } else {
                    LinkTarget::Unfollowed(Unfollowed::Address(uri.to_string()))
                }
            }
            b"Named" => {
                let name = action.get(b"N").ok().and_then(|o| o.as_name().ok());
                let last = count.saturating_sub(1);
                page(match name {
                    _ if count == 0 => None,
                    Some(b"FirstPage") => Some(0),
                    Some(b"LastPage") => Some(last),
                    Some(b"NextPage") => Some((here + 1).min(last)),
                    Some(b"PrevPage") => Some(here.saturating_sub(1)),
                    Some(other) => {
                        let other = String::from_utf8_lossy(other).into_owned();
                        return LinkTarget::Unfollowed(Unfollowed::Action(other));
                    }
                    None => None,
                })
            }
            b"GoToR" | b"GoToE" => LinkTarget::Unfollowed(Unfollowed::File(file())),
            b"Launch" => LinkTarget::Unfollowed(Unfollowed::Program(file())),
            b"JavaScript" => LinkTarget::Unfollowed(Unfollowed::Script),
            other => {
                let other = String::from_utf8_lossy(other).into_owned();
                LinkTarget::Unfollowed(Unfollowed::Action(other))
            }
        }
    }

    /// The file an action names (`/F`, a string or a file specification; for `Launch` also
    /// Windows' `/Win /F`).
    fn file_named(&self, action: &Dictionary) -> Option<String> {
        let spec = |o: &Object| -> Option<String> {
            let (_, o) = self.doc.dereference(o).ok()?;
            match o {
                Object::String(..) => o.as_str().ok().map(decode_text),
                Object::Dictionary(d) => [&b"UF"[..], b"F"]
                    .iter()
                    .find_map(|k| d.get(k).ok()?.as_str().ok().map(decode_text)),
                _ => None,
            }
        };
        let win = || {
            let (_, w) = self.doc.dereference(action.get(b"Win").ok()?).ok()?;
            spec(w.as_dict().ok()?.get(b"F").ok()?)
        };
        action
            .get(b"F")
            .ok()
            .and_then(spec)
            .or_else(win)
            .map(|s| s.trim().to_string())
    }

    /// Whether a link is drawn underlined: border style `/U` with a width above 0.
    pub(crate) fn link_underlined(&self, d: &Dictionary) -> bool {
        let Some(bs) = d.get(b"BS").ok().and_then(|o| self.dict_of(o)) else {
            return false;
        };
        let w = bs.get(b"W").ok().and_then(|o| self.num(o)).unwrap_or(1.0);
        bs.get(b"S").and_then(|o| o.as_name()).ok() == Some(b"U") && w > 0.0
    }

    /// Add a link to page `page_index` (zero-based) over `areas`, in displayed-page points
    /// (y down): one area a line for a link over text, or one box. It goes to `to`, a page of
    /// this document or a web or email address ([`web_link`], [`email_link`]), and is shown
    /// with a blue underline under each area when `underline` is set, otherwise not at all.
    /// Returns the new link's id.
    pub fn add_link(
        &mut self,
        page_index: usize,
        areas: &[[f32; 4]],
        to: &LinkTarget,
        underline: bool,
    ) -> Result<(u32, u16)> {
        let areas: Vec<[f32; 4]> = areas
            .iter()
            .copied()
            .filter(|b| b[2] > b[0] && b[3] > b[1])
            .collect();
        if areas.is_empty() {
            bail!("no area for the link");
        }
        let target = self.target_entry(to)?;
        let page = self.page_id_at(page_index)?;
        let (to_user, _, _) = self.vis_down(page);
        // Each area's corners in Acrobat's order (top left, top right, bottom left, bottom
        // right as the page is shown), as text markup's are.
        let mut quads = Vec::with_capacity(areas.len() * 8);
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for b in &areas {
            for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
                let (u, v) = mx::apply(to_user, x, y);
                quads.extend([u, v]);
                (r[0], r[1]) = (r[0].min(u), r[1].min(v));
                (r[2], r[3]) = (r[2].max(u), r[3].max(v));
            }
        }
        let mut d = dictionary! {
            "Subtype" => "Link",
            "Rect" => nums(&r),
            "QuadPoints" => nums(&quads),
            "H" => "I",
            "F" => PRINT,
        };
        d.set(target.0, target.1);
        set_look(&mut d, underline);
        let id = self.insert_annot(page, d, "")?;
        self.draw_link(id, underline)?;
        Ok(id)
    }

    /// Point a link on page `page_index` (zero-based) somewhere else and set its look, as
    /// [`Pdf::add_link`] makes them; a link of another kind (another file, a program) loses
    /// what it did. Returns the link's id.
    pub fn set_link(
        &mut self,
        page_index: usize,
        link: LinkId,
        to: &LinkTarget,
        underline: bool,
    ) -> Result<(u32, u16)> {
        let target = self.target_entry(to)?;
        self.atomically(|p| {
            let id = p.link_object(page_index, link)?;
            let d = p.doc.get_dictionary_mut(id)?;
            d.remove(b"Dest");
            d.remove(b"A");
            d.remove(b"PA");
            d.set(target.0, target.1);
            set_look(d, underline);
            d.set("M", Object::string_literal(crate::annot_write::pdf_now()));
            p.draw_link(id, underline)?;
            Ok(id)
        })
    }

    /// Move a link on page `page_index` (zero-based) so its box becomes `to` (displayed-page
    /// points, y down), its areas stretched to match. Returns the link's id.
    pub fn move_link(
        &mut self,
        page_index: usize,
        link: LinkId,
        to: [f32; 4],
    ) -> Result<(u32, u16)> {
        self.atomically(|p| {
            let id = p.link_object(page_index, link)?;
            p.move_annotation(page_index, id, to)?;
            let underline = p.link_underlined(p.doc.get_dictionary(id)?);
            p.draw_link(id, underline)?;
            Ok(id)
        })
    }

    /// Remove links from page `page_index` (zero-based). Returns how many went.
    pub fn remove_links(&mut self, page_index: usize, links: &[LinkId]) -> Result<usize> {
        self.atomically(|p| {
            let mut gone = HashSet::new();
            for &l in links {
                gone.insert(p.link_object(page_index, l)?);
            }
            p.drop_annotations(gone)
        })
    }

    /// The object of a link, made one first if it is written inside its page's list.
    fn link_object(&mut self, page_index: usize, link: LinkId) -> Result<ObjectId> {
        let id = match link {
            LinkId::Object(id) => id,
            LinkId::Inline { page, at } => {
                if page != page_index {
                    bail!("the link isn't on page {}", page_index + 1);
                }
                self.own_direct_annots()?;
                let page = self.page_id_at(page)?;
                self.annots_array(page)
                    .and_then(|a| a.get(at)?.as_reference().ok())
                    .ok_or_else(|| anyhow!("the link is no longer there"))?
            }
        };
        let d = self
            .doc
            .get_dictionary(id)
            .map_err(|_| anyhow!("the link is no longer there"))?;
        if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Link") {
            bail!("that isn't a link");
        }
        Ok(id)
    }

    /// The entry that sends a link to `to`: `/Dest` for a page (its top, the zoom left as it
    /// is, as bookmarks go), `/A` with a URI action for an address.
    fn target_entry(&self, to: &LinkTarget) -> Result<(&'static str, Object)> {
        match to {
            LinkTarget::Page(n) => {
                let Some(&page) = self.page_ids().get(*n) else {
                    bail!(
                        "page {} does not exist (document has {})",
                        n + 1,
                        self.page_count()
                    );
                };
                let top = self.page_box(page)[3];
                let dest = vec![
                    page.into(),
                    "XYZ".into(),
                    Object::Null,
                    top.into(),
                    Object::Null,
                ];
                Ok(("Dest", dest.into()))
            }
            LinkTarget::Uri(u) => {
                let uri = checked(u)?;
                let action = dictionary! {
                    "S" => "URI",
                    "URI" => Object::String(uri.into_bytes(), StringFormat::Literal),
                };
                Ok(("A", action.into()))
            }
            LinkTarget::Unfollowed(_) => bail!(ONLY_WEB_AND_EMAIL),
        }
    }

    /// Give an underlined link its drawing, LimarePDF's underline in its colour; take the
    /// drawing away from one that isn't, so nothing shows.
    fn draw_link(&mut self, id: ObjectId, underline: bool) -> Result<()> {
        if underline {
            self.draw_annotations(&[id]);
        } else {
            self.doc.get_dictionary_mut(id)?.remove(b"AP");
        }
        Ok(())
    }
}

/// A link's look: a blue underline (`/BS /S /U`), or no border at all.
fn set_look(d: &mut Dictionary, underline: bool) {
    if underline {
        d.set("BS", dictionary! { "W" => 1, "S" => "U" });
        d.set("C", nums(&LINK_BLUE));
    } else {
        d.set("BS", dictionary! { "W" => 0, "S" => "S" });
        d.remove(b"C");
    }
    let width = if underline { 1.0 } else { 0.0 };
    d.set("Border", nums(&[0.0, 0.0, width]));
}

/// The box round each quadrilateral of `/QuadPoints`, in displayed-page points.
fn quad_boxes(to_vis: [f32; 6], q: &[f32]) -> Vec<[f32; 4]> {
    q.as_chunks::<8>()
        .0
        .iter()
        .take(MAX_LINKS)
        .map(|c| {
            let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for &[px, py] in c.as_chunks::<2>().0 {
                let (x, y) = mx::apply(to_vis, px, py);
                (b[0], b[1]) = (b[0].min(x), b[1].min(y));
                (b[2], b[3]) = (b[2].max(x), b[3].max(y));
            }
            b
        })
        .filter(|b| b[2] > b[0] && b[3] > b[1])
        .collect()
}
