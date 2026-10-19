//! When a tab's recovery copy is written (the design's section 4): about a minute after the
//! first change since the last copy, at most once a minute, at once after a withdrawal
//! (content removed), one write at a time, and never for a document that has had a
//! password. One per tab; the app asks it every frame.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long after the first change a copy is written, and the least time between the
/// starts of two writes.
pub const INTERVAL: Duration = Duration::from_secs(60);

/// What [`Schedule::next`] says to do.
#[derive(Debug, PartialEq)]
pub enum Next {
    /// Start a write now.
    Now,
    /// Ask again after this long.
    After(Duration),
    /// Nothing to write until the tab changes or a write finishes.
    Idle,
}

#[derive(Debug, Default)]
pub struct Schedule {
    /// The revision the tab's copy holds; `None` for no copy.
    pub copied: Option<u64>,
    /// When a change not yet copied was first seen.
    changed: Option<Instant>,
    /// When the last write started (or was tried and got no ticket).
    started: Option<Instant>,
    /// A write is queued or running.
    pub writing: bool,
    /// Write as soon as possible: the copy was withdrawn before content was removed.
    soon: bool,
    /// The document was opened with a password or has had password settings: no copies
    /// for the rest of the session (section 4).
    pub protected: bool,
    /// A write was queued for this tab, so it may have files to withdraw.
    pub asked: bool,
    /// A failed write has been told; not again until a copy is written or the tab saved.
    pub told: bool,
    /// Files a withdrawal invalidated but couldn't delete, until they are gone.
    pub leftovers: Vec<PathBuf>,
}

impl Schedule {
    /// Whether to write the tab now, at `revision`, `dirty` when it has unsaved changes.
    pub fn next(&mut self, now: Instant, revision: u64, dirty: bool) -> Next {
        if self.protected || self.writing {
            return Next::Idle;
        }
        if !dirty || self.copied == Some(revision) {
            self.changed = None;
            return Next::Idle;
        }
        let changed = *self.changed.get_or_insert(now);
        let at = if self.soon {
            now
        } else {
            let after = changed + INTERVAL;
            self.started.map_or(after, |s| after.max(s + INTERVAL))
        };
        match at.saturating_duration_since(now) {
            Duration::ZERO => Next::Now,
            d => Next::After(d),
        }
    }

    /// A write was queued now.
    pub fn queued(&mut self, now: Instant) {
        self.tried(now);
        self.writing = true;
        self.asked = true;
    }

    /// A write was due now but couldn't be queued (no ticket): the next try waits a
    /// minute, as after a failed write.
    pub fn tried(&mut self, now: Instant) {
        self.started = Some(now);
        self.soon = false;
    }

    /// The tab's copies were withdrawn: a new copy is due at once.
    pub fn withdrawn(&mut self) {
        self.copied = None;
        self.soon = true;
    }

    /// The write finished with a copy of `revision`.
    pub fn written(&mut self, revision: u64) {
        self.writing = false;
        self.copied = Some(revision);
        self.changed = None;
        self.told = false;
    }

    /// The write was refused or failed: the tab stays due, on the usual schedule.
    pub fn not_written(&mut self) {
        self.writing = false;
    }

    /// The copy was removed (the tab was saved).
    pub fn removed(&mut self) {
        self.copied = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn changes_are_coalesced_into_one_write_a_minute_after_the_first() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        assert_eq!(s.next(t0, 0, false), Next::Idle);
        assert_eq!(s.next(t0, 1, true), Next::After(INTERVAL));
        // More changes within the minute don't move the time or queue writes of their own.
        assert_eq!(s.next(t0 + secs(20), 2, true), Next::After(secs(40)));
        assert_eq!(s.next(t0 + secs(50), 5, true), Next::After(secs(10)));
        assert_eq!(s.next(t0 + secs(60), 5, true), Next::Now);
        s.queued(t0 + secs(60));
        // One write at a time.
        assert_eq!(s.next(t0 + secs(61), 6, true), Next::Idle);
        s.written(5);
        assert_eq!(s.copied, Some(5));
    }

    #[test]
    fn a_change_during_a_write_leaves_the_tab_due() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.next(t0, 1, true);
        s.queued(t0 + secs(60));
        // Revision 2 was made while revision 1 was written.
        s.written(1);
        assert_eq!(s.next(t0 + secs(70), 2, true), Next::After(secs(60)));
        assert_eq!(s.next(t0 + secs(130), 2, true), Next::Now);
        // Nothing to do once the copy holds the tab's revision.
        s.queued(t0 + secs(130));
        s.written(2);
        assert_eq!(s.next(t0 + secs(500), 2, true), Next::Idle);
    }

    #[test]
    fn a_failed_write_is_tried_again_no_sooner_than_a_minute_later() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.next(t0, 1, true);
        s.queued(t0 + secs(60));
        s.not_written();
        assert_eq!(s.next(t0 + secs(61), 1, true), Next::After(secs(59)));
        assert_eq!(s.next(t0 + secs(120), 1, true), Next::Now);
        // No ticket (recovery blocked for the tab): the same wait.
        s.tried(t0 + secs(120));
        assert_eq!(s.next(t0 + secs(121), 1, true), Next::After(secs(59)));
    }

    #[test]
    fn a_withdrawal_makes_a_new_copy_due_at_once() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.next(t0, 1, true);
        s.queued(t0 + secs(60));
        s.written(1);
        // Redaction applied 5 s later: withdrawn, then the edit (revision 2).
        s.withdrawn();
        assert_eq!(s.copied, None);
        assert_eq!(s.next(t0 + secs(65), 2, true), Next::Now);
        // A failure of that write isn't retried faster than the schedule.
        s.queued(t0 + secs(65));
        s.not_written();
        assert_eq!(s.next(t0 + secs(66), 2, true), Next::After(secs(59)));
    }

    #[test]
    fn a_write_queued_before_a_withdrawal_is_followed_by_a_new_one() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.next(t0, 1, true);
        s.queued(t0 + secs(60));
        s.withdrawn();
        assert_eq!(s.next(t0 + secs(61), 2, true), Next::Idle);
        // The store refuses the earlier write; the new one goes at once.
        s.not_written();
        assert_eq!(s.next(t0 + secs(62), 2, true), Next::Now);
    }

    #[test]
    fn protected_and_saved_tabs_are_never_written() {
        let t0 = Instant::now();
        let mut s = Schedule {
            protected: true,
            ..Schedule::default()
        };
        assert_eq!(s.next(t0, 1, true), Next::Idle);
        s.withdrawn();
        assert_eq!(s.next(t0 + secs(600), 2, true), Next::Idle);

        let mut s = Schedule::default();
        s.next(t0, 1, true);
        // Saved before the minute was up.
        assert_eq!(s.next(t0 + secs(30), 1, false), Next::Idle);
        assert_eq!(s.next(t0 + secs(90), 1, false), Next::Idle);
        // A change after the save starts a new minute.
        assert_eq!(s.next(t0 + secs(100), 2, true), Next::After(INTERVAL));
    }
}
