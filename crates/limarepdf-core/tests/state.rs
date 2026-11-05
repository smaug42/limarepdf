//! `Pdf::state_since`, the tests' one meaning of "the document is unchanged" (cycle plan
//! section 3, rule 3), as the integration tests reach it (through the `test-seams` feature,
//! core's own dev-dependency). Its own cases are `state.rs`'s unit tests.

use limarepdf_core::{Pdf, StateDifference};

#[test]
fn a_refused_operation_leaves_the_document_unchanged() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let before = p.state_before();
    assert!(p.insert_blank(5, 300.0, 200.0).is_err());
    assert_eq!(p.state_since(&before), []);
}

#[test]
fn an_operation_that_succeeds_is_seen() {
    let mut p = Pdf::new();
    let before = p.state_before();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let changes = p.state_since(&before);
    assert!(
        changes
            .iter()
            .any(|d| matches!(d, StateDifference::Object(_))),
        "{changes:?}"
    );
}
