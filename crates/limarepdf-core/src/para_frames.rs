//! The frames of the paragraphs LimarePDF set (NEW-15, the user's finding in batch 49's
//! stage 1): a PDF keeps no paragraph's edges, so a paragraph's frame is read from where its
//! lines are, and one centred or made narrower than its frame was read narrower each time,
//! each alignment after moving it on. A paragraph whose edges or alignment were set, or
//! whose frame was read from here, is recorded on its page as it was written, in the page's
//! private data for LimarePDF (`/PieceInfo /LimarePDF /Private /Frames`, ISO 32000's
//! page-piece dictionary, which other apps leave alone), with the page's date of change the
//! standard asks for beside it; and its frame is read from here again while its lines are
//! where they were written ([`kept_for`]). Remove hidden data's private data takes it away.
//!
//! Positions are in points on the displayed page (y down), as [`crate::Block`]'s.

use crate::paragraph::Block;
use crate::{Pdf, Result};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, StringFormat};

/// The private data's key in the page's `/PieceInfo`.
const KEY: &[u8] = b"LimarePDF";

/// The most frames a page keeps, the oldest dropped first.
const MOST: usize = 500;

/// How far a line's baseline may be from where its frame was recorded, and its ends past
/// the frame's edges, for the frame to be its paragraph's still (points).
const NEAR: f32 = 0.5;
const PAST: f32 = 1.0;

/// A paragraph's frame as LimarePDF wrote it: its first and last lines' baselines, its left
/// and right edges, and its first line's indent (the paragraph's own, kept while it is
/// centred or right-aligned).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KeptFrame {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    pub indent: f32,
}

impl KeptFrame {
    fn of(nums: &[Object]) -> Option<Self> {
        let n: Vec<f32> = nums.iter().filter_map(|o| o.as_float().ok()).collect();
        let [top, bottom, left, right, indent] = n[..] else {
            return None;
        };
        let all = [top, bottom, left, right, indent];
        (all.iter().all(|v| v.is_finite()) && left < right && top <= bottom).then_some(Self {
            top,
            bottom,
            left,
            right,
            indent,
        })
    }

    fn object(&self) -> Object {
        let nums = [self.top, self.bottom, self.left, self.right, self.indent];
        Object::Array(nums.iter().map(|&v| Object::Real(round(v))).collect())
    }
}

/// `v` to a hundredth of a point, as the frame is written.
fn round(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// The dictionary `o` is or refers to in `doc`.
fn dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    doc.dereference(o).ok()?.1.as_dict().ok()
}

/// The array `o` is or refers to in `doc`.
fn array<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Vec<Object>> {
    doc.dereference(o).ok()?.1.as_array().ok()
}

/// The frame of `kept` that block `block` was written in, if its lines are still where it
/// was written: its first and last baselines where they were, and every line within its
/// edges.
pub(crate) fn kept_for(block: &Block, kept: &[KeptFrame]) -> Option<KeptFrame> {
    let (first, last) = (block.lines.first()?, block.lines.last()?);
    let within = |k: &KeptFrame| {
        (block.lines.iter()).all(|l| l.bbox[0] >= k.left - PAST && l.bbox[2] <= k.right + PAST)
    };
    (kept.iter().rev())
        .find(|k| {
            (first.baseline - k.top).abs() <= NEAR
                && (last.baseline - k.bottom).abs() <= NEAR
                && within(k)
        })
        .copied()
}

impl Pdf {
    /// The frames recorded on page `page` ([`KeptFrame`]), oldest first; none where the
    /// record is missing or not as LimarePDF writes it.
    pub(crate) fn kept_frames(&self, page: ObjectId) -> Vec<KeptFrame> {
        let doc = &self.doc;
        let frames = (|| {
            let page = doc.get_dictionary(page).ok()?;
            let pieces = dict(doc, page.get(b"PieceInfo").ok()?)?;
            let ours = dict(doc, pieces.get(KEY).ok()?)?;
            let private = dict(doc, ours.get(b"Private").ok()?)?;
            array(doc, private.get(b"Frames").ok()?)
        })();
        (frames.into_iter().flatten())
            .filter_map(|o| KeptFrame::of(array(doc, o)?))
            .collect()
    }

    /// Record `frame` on page `page` for a paragraph written there, in place of those of
    /// the lines it overlaps, there now or before (from `was`'s first baseline to its last),
    /// and date the page's change, as the standard asks beside private data. Other apps'
    /// private data on the page stays.
    pub(crate) fn keep_frame(
        &mut self,
        page: ObjectId,
        frame: KeptFrame,
        was: (f32, f32),
    ) -> Result<()> {
        let mut frames = self.kept_frames(page);
        let (top, bottom) = (frame.top.min(was.0), frame.bottom.max(was.1));
        frames.retain(|k| {
            k.bottom < top - NEAR
                || k.top > bottom + NEAR
                || k.right < frame.left
                || k.left > frame.right
        });
        frames.push(frame);
        let extra = frames.len().saturating_sub(MOST);
        frames.drain(..extra);
        let now = Object::String(
            crate::annot_write::pdf_now().into_bytes(),
            StringFormat::Literal,
        );
        let mut pieces = {
            let page = self.doc.get_dictionary(page)?;
            match page.get(b"PieceInfo") {
                Ok(o) => (self.doc.dereference(o).ok())
                    .and_then(|(_, o)| o.as_dict().ok().cloned())
                    .unwrap_or_default(),
                Err(_) => Dictionary::new(),
            }
        };
        pieces.set(
            KEY,
            dictionary! {
                "LastModified" => now.clone(),
                "Private" => dictionary! {
                    "Frames" => frames.iter().map(KeptFrame::object).collect::<Vec<_>>(),
                },
            },
        );
        let page = self.doc.get_dictionary_mut(page)?;
        page.set("PieceInfo", pieces);
        page.set("LastModified", now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_read_only_as_written() {
        let r = |v: f32| Object::Real(v);
        let good = [r(100.0), r(160.0), r(72.0), r(512.0), r(18.0)];
        let k = KeptFrame::of(&good).expect("a frame");
        assert_eq!((k.left, k.right, k.indent), (72.0, 512.0, 18.0));
        assert!(KeptFrame::of(&good[..4]).is_none(), "four numbers");
        let backwards = [r(100.0), r(160.0), r(512.0), r(72.0), r(0.0)];
        assert!(KeptFrame::of(&backwards).is_none(), "edges backwards");
        let name = Object::Name(b"x".to_vec());
        let named = [r(100.0), r(160.0), r(72.0), name, r(0.0)];
        assert!(KeptFrame::of(&named).is_none(), "not a number");
    }
}
