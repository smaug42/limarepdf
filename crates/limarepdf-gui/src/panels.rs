//! Whether a side panel is open: what the user chose, kept apart from a narrow window
//! folding the panel away for now.

/// A side panel's state. The user's choice is remembered between runs; a window too
/// narrow for the panel folds it away without changing that choice, and it comes back
/// when there is room again. Opening it while the window is narrow keeps it open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    /// The user wants it open.
    want: bool,
    /// The window is too narrow for it.
    narrow: bool,
    /// Opened by the user while the window was narrow: shown despite that.
    kept: bool,
}

impl Panel {
    pub fn new(want: bool) -> Self {
        Self {
            want,
            narrow: false,
            kept: false,
        }
    }

    /// Whether it is shown now.
    pub fn shown(self) -> bool {
        self.want && (!self.narrow || self.kept)
    }

    /// What the user chose, to remember.
    pub fn wanted(self) -> bool {
        self.want
    }

    /// The window's width changed whether there is room. Becoming narrow folds the panel
    /// away, even one opened while narrow before.
    pub fn set_narrow(&mut self, narrow: bool) {
        if narrow && !self.narrow {
            self.kept = false;
        }
        self.narrow = narrow;
    }

    /// The user opened or closed it.
    pub fn set_open(&mut self, open: bool) {
        self.want = open;
        self.kept = open && self.narrow;
    }

    /// The user opened it if hidden, or closed it if shown.
    pub fn toggle(&mut self) {
        self.set_open(!self.shown());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_by_hand_survives_resizing() {
        let mut p = Panel::new(true);
        p.set_open(false);
        p.set_narrow(true);
        p.set_narrow(false);
        assert!(!p.shown());
        assert!(!p.wanted());
    }

    #[test]
    fn a_narrow_window_folds_it_away_until_there_is_room() {
        let mut p = Panel::new(true);
        p.set_narrow(true);
        assert!(!p.shown());
        // Folding away isn't the user's choice: it is still remembered as open.
        assert!(p.wanted());
        p.set_narrow(false);
        assert!(p.shown());
    }

    #[test]
    fn opening_it_in_a_narrow_window_keeps_it_open() {
        let mut p = Panel::new(true);
        p.set_narrow(true);
        p.toggle();
        assert!(p.shown());
        // The next frame's width check doesn't undo it.
        p.set_narrow(true);
        assert!(p.shown());
        p.set_narrow(false);
        assert!(p.shown());
        // Narrowing again later folds it away as usual.
        p.set_narrow(true);
        assert!(!p.shown());
    }

    #[test]
    fn closed_at_start_stays_closed_when_wide() {
        let mut p = Panel::new(false);
        p.set_narrow(false);
        assert!(!p.shown());
        p.toggle();
        assert!(p.shown() && p.wanted());
    }
}
