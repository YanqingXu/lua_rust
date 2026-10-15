#[path = "support/repl_byte_contract.rs"]
mod contract;

#[test]
fn corpus_definition_keeps_raw_boundaries_and_explicit_banner() {
    let cases = contract::cases();
    assert_eq!(cases.len(), 26);
    for (id, input) in [
        ("idle-eof", b"".as_slice()),
        ("final-expression-no-lf", b"=40+2".as_slice()),
        (
            "short-string-lf-invalid",
            b"return 'broken\n=42\n".as_slice(),
        ),
        ("short-string-no-lf-pending", b"return 'broken".as_slice()),
        ("crlf-expression", b"=40+2\r\n".as_slice()),
        ("crlf-continuation", b"return 40 +\r\n2\r\n".as_slice()),
        ("crlf-blank", b"\r\n".as_slice()),
        ("crlf-pending-eof", b"if true then\r\n".as_slice()),
        (
            "final-block-end-no-lf",
            b"if true then\nreturn 42\nend".as_slice(),
        ),
    ] {
        assert_eq!(
            cases.iter().find(|case| case.id == id).unwrap().stdin,
            input
        );
    }
    let binary = cases
        .iter()
        .find(|case| case.id == "nil-empty-binary-values")
        .unwrap();
    assert!(binary
        .stdout(env!("CARGO_PKG_VERSION"))
        .windows(2)
        .any(|bytes| bytes == [0, 255]));
    for case in &cases {
        assert_eq!(
            case.stdout(env!("CARGO_PKG_VERSION"))
                .starts_with(contract::banner(env!("CARGO_PKG_VERSION"))),
            case.args == ["-i"]
        );
    }
}

#[test]
fn real_binary_matches_all_shared_raw_byte_cases() {
    contract::verify(
        std::path::Path::new(env!("CARGO_BIN_EXE_lua_app")),
        env!("CARGO_PKG_VERSION"),
    );
}
