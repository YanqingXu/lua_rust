use lua_compiler::parser::{ParseError, Parser};

fn parse(source: &str) -> Result<(), ParseError> {
    let mut parser = Parser::from_bytes(source.as_bytes());
    parser.parse().map(|_| ())
}

fn nested_parentheses(depth: usize) -> String {
    format!("local x = {}42{}", "(".repeat(depth), ")".repeat(depth))
}

fn nested_if(depth: usize) -> String {
    format!(
        "{}local x = 1\n{}",
        "if true then\n".repeat(depth),
        "end\n".repeat(depth)
    )
}

fn local_names(count: usize) -> Vec<String> {
    (1..=count).map(|index| format!("v{index}")).collect()
}

fn upvalue_source(count: usize) -> String {
    let names = local_names(count);
    format!(
        "function outer()\nlocal {}\nreturn function() return {} end\nend\n",
        names.join(","),
        names.join(",")
    )
}

#[test]
fn expression_and_block_recursion_limits_match_the_cpp_parser() {
    parse(&nested_parentheses(90)).expect("90 nested parentheses are inside the oracle limit");
    let error = parse(&nested_parentheses(150)).expect_err("150 nested parentheses exceed limit");
    assert_eq!(error.message, "chunk has too many syntax levels");
    assert_eq!(error.line, 1);

    parse(&nested_if(79)).expect("79 nested blocks are inside the oracle limit");
    let error = parse(&nested_if(80)).expect_err("80 nested blocks reach the guarded limit");
    assert_eq!(error.message, "chunk has too many syntax levels");
    assert_eq!(error.line, 81);
}

#[test]
fn right_associative_and_call_recursion_fail_structurally() {
    let concat = format!("return {}", vec!["a"; 400].join(" .. "));
    let error = parse(&concat).expect_err("400-item concat chain exceeds limit");
    assert_eq!(error.message, "chunk has too many syntax levels");
    assert_eq!(error.line, 1);

    let call = format!("return {}1{}", "f(".repeat(400), ")".repeat(400));
    let error = parse(&call).expect_err("400 nested call arguments exceed limit");
    assert_eq!(error.message, "chunk has too many syntax levels");
    assert_eq!(error.line, 1);
}

#[test]
fn local_and_upvalue_caps_keep_the_cpp_count_and_function_line() {
    parse(&format!("local {}", local_names(200).join(","))).expect("200 locals are accepted");
    let error = parse(&format!("local {}", local_names(201).join(",")))
        .expect_err("201 locals exceed the Lua 5.1 cap");
    assert_eq!(
        error.message,
        "function at line 1 has more than 200 local variables"
    );
    assert_eq!(error.line, 1);

    parse(&upvalue_source(60)).expect("60 upvalues are accepted");
    let error = parse(&upvalue_source(61)).expect_err("61 upvalues exceed the cap");
    assert_eq!(
        error.message,
        "function at line 3 has more than 60 upvalues"
    );
    assert_eq!(error.line, 3);
}

#[test]
fn malformed_method_call_reports_the_cpp_token_category() {
    let error =
        parse("return a:()").expect_err("a method call requires an identifier after the colon");
    assert_eq!(error.message, "Expected method name after ':' near '('");
    assert_eq!(error.line, 1);
}
