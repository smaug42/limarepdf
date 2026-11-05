//! Files being written, reserved until their writers end (S1-03: H-05, PB-86). A save, a
//! background job and signing claim the files they write as they start, and until each
//! ends, nothing else in the app writes those files: two writers to one file would leave it
//! holding either one's, or a mix. Writers are admitted and claim on the window's thread,
//! so nothing comes between the check and the claim. A file written at once on the window's
//! thread (the text exported, the selection saved) is checked but never claimed: nothing
//! else runs while it is written.
//!
//! The claims compare names without case, as a disk that ignores case does; another tab's
//! open file is told exactly, by its path or (on Unix) its inode. Not covered: exporting a
//! digital ID or certificate, the recovery store's private folder, and other apps.

use super::App;
use limarepdf_core::disk::{self, Identity};
use std::path::{Path, PathBuf};

/// What claimed a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Owner {
    /// Tab `tab`'s save number `n`.
    Save { tab: u64, n: u64 },
    /// Background job number `id`.
    Job { id: u64 },
    /// The signing window's signing (there is one window at most).
    Signing,
}

/// The files claimed, with their writers.
#[derive(Default)]
pub(super) struct Destinations {
    claims: Vec<(Identity, Owner)>,
}

impl Destinations {
    /// `owner` writes `paths` until it is released.
    pub(super) fn claim(&mut self, paths: &[PathBuf], owner: Owner) {
        let claimed = paths.iter().map(|p| (disk::identity(p), owner));
        self.claims.extend(claimed);
    }

    /// `owner` ended: its files are free again.
    pub(super) fn release(&mut self, owner: Owner) {
        self.claims.retain(|(_, o)| *o != owner);
    }

    /// What is writing the file at `path`, if anything.
    fn holder(&self, path: &Path) -> Option<Owner> {
        let want = disk::identity(path);
        let mut held = self
            .claims
            .iter()
            .filter(|(i, _)| i.matches_ignoring_case(&want));
        held.next().map(|(_, o)| *o)
    }

    /// Whether any save's file is still claimed.
    pub(super) fn saving(&self) -> bool {
        self.claims
            .iter()
            .any(|(_, o)| matches!(o, Owner::Save { .. }))
    }
}

impl App {
    /// Why tab `tab` may not write the file at `path`, with the message's title: it is open
    /// in a tab (other than `tab`, when `own_file`, as a tab saves over its own file), or
    /// it is being written.
    pub(super) fn conflict(
        &self,
        path: &Path,
        tab: u64,
        own_file: bool,
    ) -> Option<(&'static str, String)> {
        if let Some(why) = self.in_other_tab(path, own_file.then_some(tab)) {
            let title = if own_file {
                "Open in another tab"
            } else {
                "Open in a tab"
            };
            return Some((title, why));
        }
        let holder = self.destinations.holder(path)?;
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        Some(match holder {
            Owner::Save { tab: from, .. } => {
                let which = if from == tab { "this" } else { "another" };
                (
                    "Being saved",
                    format!(
                        "\"{name}\" is being saved from {which} tab. Wait until it is saved, or \
                         choose another name. Nothing was saved."
                    ),
                )
            }
            Owner::Job { .. } | Owner::Signing => (
                "Being written",
                format!(
                    "\"{name}\" is being written. Wait until it is written, or choose another \
                     name. Nothing was saved."
                ),
            ),
        })
    }

    /// The first of `paths` tab `tab` may not write, saying why ([`Self::conflict`]).
    pub(super) fn conflicts(
        &self,
        paths: &[PathBuf],
        tab: u64,
        own_file: bool,
    ) -> Option<(&'static str, String)> {
        paths.iter().find_map(|p| self.conflict(p, tab, own_file))
    }
}
