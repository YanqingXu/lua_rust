const IO_SOURCE: &str = include_str!("../src/io.rs");

#[test]
fn io_handle_borrows_are_exposed_only_through_hrtb_callbacks() {
    for forbidden in [
        "fn file_state(",
        "fn file_state_mut(",
        "fn file_data_mut(",
        "Option<&'static Table>",
        "Option<&'static mut Table>",
        "Option<&'static mut IoFileData>",
        "FnOnce(&'state mut Table)",
    ] {
        assert!(
            !IO_SOURCE.contains(forbidden),
            "IO handle access regressed to an escaping borrow: {forbidden}"
        );
    }

    for required in [
        "fn with_file_state<R>(",
        "impl for<'state> FnOnce(&'state Table) -> R",
        "fn file_state_ref(",
        "transaction.protect(table)?",
        "transaction.set_table_string(&table, &key, &text)",
        "fn with_file_data<R>(",
        "impl for<'data> FnOnce(&'data IoFileData) -> R",
        "fn with_file_data_mut<R>(",
        "impl for<'data> FnOnce(&'data mut IoFileData) -> R",
    ] {
        assert!(
            IO_SOURCE.contains(required),
            "missing scoped IO handle accessor contract: {required}"
        );
    }
}
