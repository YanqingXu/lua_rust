const EXECUTE_SOURCE: &str = include_str!("../src/execute.rs");

#[test]
fn execute_does_not_bypass_collector_scoped_access() {
    for forbidden in [".as_ref()", ".as_mut()"] {
        assert!(
            !EXECUTE_SOURCE.contains(forbidden),
            "execute.rs must keep managed-object access inside GarbageCollector scoped callbacks; found {forbidden}"
        );
    }
}
