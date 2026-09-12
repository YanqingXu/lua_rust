use std::process::Command;

fn run_lua(source: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(["-e", source])
        .output()
        .expect("lua_app should start");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).expect("number output is UTF-8")
}

#[test]
fn lua_number_text_matches_the_oracles_across_output_and_coercion_routes() {
    let output = run_lua(include_str!(
        "../../../tests/characterization/number-format-matrix.lua"
    ));
    let expected_values = [
        "3.3333333333333",
        "1e-05",
        "0.0001",
        "10000000000000",
        "1e+14",
        "1e+20",
        "1e-308",
        "4.9406564584125e-324",
        "0",
        "-0",
        "inf",
        "-inf",
        if cfg!(windows) { "-nan(ind)" } else { "-nan" },
        "1e+14",
        "0.0001",
        "nan",
    ];
    let mut expected = Vec::new();
    for (index, value) in expected_values.iter().enumerate() {
        expected.push(format!(
            "{}\t{value}\t{value}\t{value}\t{value}\t{value}",
            index + 1
        ));
    }
    for (index, value) in expected_values.iter().enumerate() {
        expected.push(format!("io\t{}\t{value}", index + 1));
    }
    expected.push("number conversion routes passed".into());
    assert_eq!(output.lines().collect::<Vec<_>>(), expected);
}

#[test]
fn general_format_uses_rounded_exponents_and_preserves_printf_flags() {
    let output = run_lua(
        r#"
        print(string.format('%.3g', 999.5))
        print(string.format('%.3G', 999.5))
        print(string.format('%#.3g', 999.5))
        print(string.format('%#.3g', 0))
        print(string.format('%+.3g', -1/math.huge))
        print(string.format('% .3g', 1))
        print(string.format('%010.3g', 12))
        print(string.format('%-10.3g', 12))
        print(string.format('%#.1g', 1))
        print(string.format('%.0g', 2.5))
        print(string.format('%010g', math.huge))
        print(string.format('%+010G', math.huge))
        print(string.format('%.14g', 99999999999999.5))
        print(string.format('%.14g', 0.0000999999999999995))
        print(string.format('%G', -math.sqrt(-1)))
    "#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        [
            "1e+03",
            "1E+03",
            "1.00e+03",
            "0.00",
            "-0",
            " 1",
            "0000000012",
            "12        ",
            "1.",
            "2",
            "       inf",
            "      +INF",
            "1e+14",
            "0.0001",
            "NAN",
        ]
    );
}
