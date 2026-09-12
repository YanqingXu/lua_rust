use std::process::Command;

#[test]
fn automatic_file_finalizers_do_not_interrupt_repeated_io() {
    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args([
            "-e",
            r#"
            collectgarbage("setpause", 100)
            collectgarbage("setstepmul", 10000)
            for i = 1, 80 do
                local file = assert(io.tmpfile())
                local text = string.rep(tostring(i), 100)
                assert(file:write(text))
                assert(file:seek("set", 0) == 0)
                assert(file:read("*a") == text)
                assert(file:close())
            end
            collectgarbage("collect")
            print("io-finalizers-complete")
        "#,
        ])
        .output()
        .expect("lua_app starts");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "io-finalizers-complete\n"
    );
}
