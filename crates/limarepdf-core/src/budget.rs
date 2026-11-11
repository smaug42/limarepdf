//! Work budgets: one for each operation that follows a document's structure wherever it
//! leads, charged across all of it, so a file built to be followed for ever (or for hours) is
//! refused with a [`TooComplex`] instead, before anything is changed.

/// The work an operation may still do.
pub(crate) struct Work {
    left: u64,
    what: &'static str,
    limit: u64,
}

/// An operation's limit, `normal` steps; a test can set another through [`budget_seam`].
pub(crate) fn limit(normal: u64) -> u64 {
    #[cfg(any(test, feature = "test-seams"))]
    if let Some(n) = budget_seam::limit() {
        return n;
    }
    normal
}

/// A test's limit ([`budget_seam`]) on the thread that hands work to others, carried to
/// them ([`Carry::run`]); outside tests, nothing.
#[derive(Clone, Copy)]
pub(crate) struct Carry {
    #[cfg(any(test, feature = "test-seams"))]
    limit: Option<u64>,
}

impl Carry {
    /// This thread's limit.
    pub(crate) fn here() -> Carry {
        Carry {
            #[cfg(any(test, feature = "test-seams"))]
            limit: budget_seam::limit(),
        }
    }

    /// Run `f` with the limit carried.
    pub(crate) fn run<T>(self, f: impl FnOnce() -> T) -> T {
        #[cfg(any(test, feature = "test-seams"))]
        return budget_seam::carried(self.limit, f);
        #[cfg(not(any(test, feature = "test-seams")))]
        f()
    }
}

impl Work {
    /// A budget of `limit` steps for `what` (as the refusal names it: "checking the
    /// drawings").
    pub(crate) fn new(what: &'static str, limit: u64) -> Work {
        Work {
            left: limit,
            what,
            limit,
        }
    }

    /// Take `n` steps; refused, with nothing taken, when fewer are left.
    pub(crate) fn take(&mut self, n: u64) -> Result<(), TooComplex> {
        if n > self.left {
            return Err(TooComplex {
                what: self.what,
                limit: self.limit,
            });
        }
        self.left -= n;
        Ok(())
    }

    /// The steps taken so far.
    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn taken(&self) -> u64 {
        self.limit - self.left
    }
}

#[cfg(any(test, feature = "test-seams"))]
impl Drop for Work {
    fn drop(&mut self) {
        budget_seam::record(self.taken());
    }
}

/// An operation was refused because the document is too complex to go through within its
/// budget; nothing was changed. Found through the error's chain (`downcast_ref`), like
/// [`crate::NotConfirmed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooComplex {
    /// What the steps were for: "checking the drawings".
    pub what: &'static str,
    /// The steps allowed.
    pub limit: u64,
}

impl std::fmt::Display for TooComplex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the document is too complex: {} would take more than {} steps, so nothing was changed",
            self.what, self.limit
        )
    }
}

impl std::error::Error for TooComplex {}

/// A test seam: operations on this thread get another limit ([`limit`]), and what each
/// budget took is kept, so a test can show a budget trips one step over what an operation
/// needs. Built only for tests (core's own, and through the `test-seams` feature).
#[cfg(any(test, feature = "test-seams"))]
pub mod budget_seam {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static LIMIT: Cell<Option<u64>> = const { Cell::new(None) };
        static TAKEN: RefCell<Option<Vec<u64>>> = const { RefCell::new(None) };
    }

    /// Runs `f` with the operations on this thread meanwhile limited to `limit` steps.
    /// Returns what `f` returned, and the steps each budget made meanwhile took, in the
    /// order they were made.
    pub fn with_limit<T>(limit: u64, f: impl FnOnce() -> T) -> (T, Vec<u64>) {
        let was = LIMIT.replace(Some(limit));
        let kept = TAKEN.replace(Some(Vec::new()));
        let out = f();
        LIMIT.set(was);
        let taken = TAKEN.replace(kept).unwrap_or_default();
        (out, taken)
    }

    pub(super) fn limit() -> Option<u64> {
        LIMIT.get()
    }

    /// Runs `f` with the operations on this thread meanwhile limited to `limit`, another
    /// thread's ([`super::Carry`]).
    pub(super) fn carried<T>(limit: Option<u64>, f: impl FnOnce() -> T) -> T {
        let was = LIMIT.replace(limit);
        let out = f();
        LIMIT.set(was);
        out
    }

    pub(super) fn record(taken: u64) {
        TAKEN.with_borrow_mut(|t| {
            if let Some(t) = t {
                t.push(taken);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_up_to_the_limit_are_taken_and_one_more_is_refused() {
        let ((), taken) = budget_seam::with_limit(5, || {
            let mut w = Work::new("counting", limit(1000));
            assert_eq!(w.take(3), Ok(()));
            assert_eq!(w.take(2), Ok(()));
            let e = w.take(1).unwrap_err();
            assert_eq!(
                e,
                TooComplex {
                    what: "counting",
                    limit: 5
                }
            );
            assert_eq!(w.taken(), 5);
        });
        assert_eq!(taken, vec![5]);
    }

    #[test]
    fn a_refusal_is_found_through_the_error_chain() {
        let mut w = Work::new("counting", 0);
        let e = anyhow::Error::from(w.take(1).unwrap_err()).context("redacting");
        let t = e.downcast_ref::<TooComplex>().expect("too complex");
        assert_eq!(t.limit, 0);
        assert!(format!("{e:#}").contains("counting would take more than 0 steps"));
    }
}
