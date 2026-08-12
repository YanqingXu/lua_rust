const TABLE_SOURCE: &str = include_str!("../src/table.rs");

#[test]
fn table_read_only_gc_handles_use_the_scoped_state_accessor() {
    let production = TABLE_SOURCE
        .split("#[cfg(test)]")
        .next()
        .expect("table source has a production prefix");

    assert!(
        !production.contains("table_ref.as_ref()"),
        "table standard library regressed to a direct GcRef<Table> dereference"
    );
    assert_eq!(
        production.matches("with_gc_ref(").count(),
        3,
        "the getn/maxn/concat scoped-access slice changed; audit the new boundary"
    );
}
