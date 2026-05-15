//! Invariant tests for [`CallerContext`].

use acos_authority_types::CallerContext;

#[test]
fn anonymous_caller_carries_unknown_sentinels() {
    let c = CallerContext::anonymous();
    assert_eq!(c.uid, CallerContext::UNKNOWN);
    assert_eq!(c.gid, CallerContext::UNKNOWN);
    assert_eq!(c.pid, CallerContext::UNKNOWN);
    assert!(c.is_anonymous());
}

#[test]
fn from_parts_preserves_components() {
    let c = CallerContext::from_parts(1000, 100, 42);
    assert_eq!(c.uid, 1000);
    assert_eq!(c.gid, 100);
    assert_eq!(c.pid, 42);
    assert!(!c.is_anonymous());
}

#[test]
fn root_caller_is_not_anonymous() {
    // uid=0 on Redox is legitimate `root`; only the sentinel u32::MAX
    // means "no identity attached". This test pins that distinction.
    let root = CallerContext::from_parts(0, 0, 1);
    assert!(!root.is_anonymous());
}

#[test]
fn label_for_anonymous_is_dash() {
    assert_eq!(CallerContext::anonymous().label(), "-");
}

#[test]
fn label_for_named_caller_shows_uid_gid_pid() {
    let c = CallerContext::from_parts(1000, 100, 42);
    assert_eq!(c.label(), "uid=1000,gid=100,pid=42");
}

#[test]
fn default_is_anonymous() {
    let c: CallerContext = Default::default();
    assert!(c.is_anonymous());
}

#[test]
fn two_distinct_callers_compare_unequal() {
    let a = CallerContext::from_parts(1000, 100, 42);
    let b = CallerContext::from_parts(1001, 100, 42);
    assert_ne!(a, b);
}
