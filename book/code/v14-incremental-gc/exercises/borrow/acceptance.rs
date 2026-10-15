mod compiler;

#[test]
#[ignore = "learner starter intentionally fails rustc; implement short borrowing first"]
fn short_borrow_scans_owned_edges() {
    compiler::assert_compiles_and_scans("starter");
}
