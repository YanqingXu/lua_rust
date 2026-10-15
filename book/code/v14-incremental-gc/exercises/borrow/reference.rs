mod compiler;

#[test]
fn long_borrow_is_rejected_with_e0499() {
    let compiled = compiler::compile("before");
    assert!(
        !compiled.output.status.success(),
        "the long borrow must be rejected"
    );
    let diagnostic = String::from_utf8(compiled.output.stderr).unwrap();
    assert!(
        diagnostic.contains("\"code\":\"E0499\""),
        "require E0499, not an unrelated compiler failure:\n{diagnostic}"
    );
    println!("before: rustc rejected the long borrow with E0499\n{diagnostic}");
}

#[test]
fn short_borrow_preserves_child_edges_and_scan_progress() {
    compiler::assert_compiles_and_scans("solution");
}
