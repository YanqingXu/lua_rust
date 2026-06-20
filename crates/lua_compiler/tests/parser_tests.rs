//! Parser integration tests
//!
//! Validate the recursive-descent parser against known Lua 5.1 inputs.

use lua_compiler::ast::expr::{BinaryOp, Expr, UnaryOp};
use lua_compiler::ast::stmt::Stmt;
use lua_compiler::parser::Parser;

/// Parse source and return the AST Chunk
fn parse(source: &str) -> lua_compiler::ast::stmt::Chunk {
    let mut parser = Parser::new(source);
    parser.parse().expect("parse should succeed")
}

/// Parse source and expect a ParseError
fn parse_error(source: &str) -> lua_compiler::parser::ParseError {
    let mut parser = Parser::new(source);
    parser.parse().expect_err("parse should fail")
}

// =====================================================================
// Expression tests
// =====================================================================

#[test]
fn test_parse_nil_call() {
    let err = parse_error("nil");
    assert!(err.message.contains("unexpected symbol"));
}

#[test]
fn test_parse_number() {
    let chunk = parse("return 42");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert_eq!(ret.values.len(), 1);
    assert!(matches!(&*ret.values[0], Expr::Number(n) if (n.value - 42.0).abs() < f64::EPSILON));
}

#[test]
fn test_parse_string() {
    let chunk = parse("return 'hello'");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::String(s) if s.value == "hello"));
}

#[test]
fn test_parse_boolean_true() {
    let chunk = parse("return true");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Boolean(b) if b.value));
}

#[test]
fn test_parse_boolean_false() {
    let chunk = parse("return false");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Boolean(b) if !b.value));
}

#[test]
fn test_parse_vararg() {
    let chunk = parse("return ...");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Vararg(_)));
}

// =====================================================================
// Binary expression tests
// =====================================================================

#[test]
fn test_parse_add() {
    let chunk = parse("return 1 + 2");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Add));
}

#[test]
fn test_parse_sub() {
    let chunk = parse("return 5 - 3");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Sub));
}

#[test]
fn test_parse_mul() {
    let chunk = parse("return 3 * 4");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Mul));
}

#[test]
fn test_parse_div() {
    let chunk = parse("return 10 / 2");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Div));
}

#[test]
fn test_parse_mod() {
    let chunk = parse("return 10 % 3");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Mod));
}

#[test]
fn test_parse_pow() {
    let chunk = parse("return 2 ^ 3");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Pow));
}

#[test]
fn test_parse_concat() {
    let chunk = parse("return 'a' .. 'b'");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Concat));
}

#[test]
fn test_parse_comparison_eq() {
    let chunk = parse("return a == b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Eq));
}

#[test]
fn test_parse_comparison_ne() {
    let chunk = parse("return a ~= b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Ne));
}

#[test]
fn test_parse_comparison_lt() {
    let chunk = parse("return a < b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Lt));
}

#[test]
fn test_parse_comparison_le() {
    let chunk = parse("return a <= b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Le));
}

#[test]
fn test_parse_comparison_gt() {
    let chunk = parse("return a > b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Gt));
}

#[test]
fn test_parse_comparison_ge() {
    let chunk = parse("return a >= b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Ge));
}

#[test]
fn test_parse_logic_and() {
    let chunk = parse("return a and b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::And));
}

#[test]
fn test_parse_logic_or() {
    let chunk = parse("return a or b");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Binary(b) if b.op == BinaryOp::Or));
}

#[test]
fn test_parse_precedence() {
    let chunk = parse("return a + b * c");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    // Should be: (+ a (* b c))
    if let Expr::Binary(bin) = &*ret.values[0] {
        assert_eq!(bin.op, BinaryOp::Add);
        assert!(matches!(&*bin.left, Expr::Name(n) if n.name == "a"));
        assert!(matches!(&*bin.right, Expr::Binary(r) if r.op == BinaryOp::Mul));
    } else {
        panic!("Expected binary expr");
    }
}

#[test]
fn test_parse_right_assoc_concat() {
    // .. is right-associative
    let chunk = parse("return 'a' .. 'b' .. 'c'");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    // Should be: (.. "a" (.. "b" "c"))
    if let Expr::Binary(bin) = &*ret.values[0] {
        assert_eq!(bin.op, BinaryOp::Concat);
        assert!(matches!(&*bin.right, Expr::Binary(r) if r.op == BinaryOp::Concat));
    } else {
        panic!("Expected binary concat");
    }
}

#[test]
fn test_parse_right_assoc_pow() {
    let chunk = parse("return 2 ^ 3 ^ 2");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    if let Expr::Binary(bin) = &*ret.values[0] {
        assert_eq!(bin.op, BinaryOp::Pow);
        assert!(matches!(&*bin.right, Expr::Binary(r) if r.op == BinaryOp::Pow));
    } else {
        panic!("Expected binary pow");
    }
}

// =====================================================================
// Unary expression tests
// =====================================================================

#[test]
fn test_parse_unary_not() {
    let chunk = parse("return not true");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Unary(u) if u.op == UnaryOp::Not));
}

#[test]
fn test_parse_unary_neg() {
    let chunk = parse("return -42");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Unary(u) if u.op == UnaryOp::Neg));
}

#[test]
fn test_parse_unary_len() {
    let chunk = parse("return #t");
    let ret = match chunk.statements[0].as_ref() {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Unary(u) if u.op == UnaryOp::Len));
}

// =====================================================================
// Statement tests
// =====================================================================

#[test]
fn test_parse_assignment() {
    let chunk = parse("x = 42");
    assert_eq!(chunk.statements.len(), 1);
    assert!(matches!(&*chunk.statements[0], Stmt::Assign(_)));
    let assign = match &*chunk.statements[0] {
        Stmt::Assign(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(assign.targets.len(), 1);
    assert_eq!(assign.values.len(), 1);
    assert!(matches!(&*assign.targets[0], Expr::Name(n) if n.name == "x"));
}

#[test]
fn test_parse_multi_assignment() {
    let chunk = parse("a, b = 1, 2");
    let assign = match &*chunk.statements[0] {
        Stmt::Assign(a) => a,
        _ => panic!(),
    };
    assert_eq!(assign.targets.len(), 2);
    assert_eq!(assign.values.len(), 2);
}

#[test]
fn test_parse_local_declaration() {
    let chunk = parse("local x");
    assert!(matches!(&*chunk.statements[0], Stmt::Local(_)));
    let local = match &*chunk.statements[0] {
        Stmt::Local(l) => l,
        _ => unreachable!(),
    };
    assert_eq!(local.names.len(), 1);
    assert_eq!(local.names[0], "x");
    assert_eq!(local.values.len(), 0);
}

#[test]
fn test_parse_local_with_init() {
    let chunk = parse("local x = 42");
    let local = match &*chunk.statements[0] {
        Stmt::Local(l) => l,
        _ => panic!(),
    };
    assert_eq!(local.names, vec!["x"]);
    assert_eq!(local.values.len(), 1);
}

#[test]
fn test_parse_multiple_locals() {
    let chunk = parse("local a, b, c = 1, 2, 3");
    let local = match &*chunk.statements[0] {
        Stmt::Local(l) => l,
        _ => panic!(),
    };
    assert_eq!(local.names, vec!["a", "b", "c"]);
    assert_eq!(local.values.len(), 3);
}

#[test]
fn test_parse_function_call() {
    let chunk = parse("print('hello')");
    assert!(matches!(&*chunk.statements[0], Stmt::Call(_)));
}

#[test]
fn test_parse_if_statement() {
    let source = "if x > 0 then return 1 end";
    let chunk = parse(source);
    let if_stmt = match &*chunk.statements[0] {
        Stmt::If(i) => i,
        _ => panic!(),
    };
    assert_eq!(if_stmt.branches.len(), 1);
    assert_eq!(if_stmt.else_branch.len(), 0);
}

#[test]
fn test_parse_if_else_statement() {
    let source = "if x > 0 then return 1 else return 0 end";
    let chunk = parse(source);
    let if_stmt = match &*chunk.statements[0] {
        Stmt::If(i) => i,
        _ => panic!(),
    };
    assert_eq!(if_stmt.else_branch.len(), 1);
}

#[test]
fn test_parse_if_elseif_statement() {
    let source = "if x == 1 then return 1 elseif x == 2 then return 2 else return 0 end";
    let chunk = parse(source);
    let if_stmt = match &*chunk.statements[0] {
        Stmt::If(i) => i,
        _ => panic!(),
    };
    assert_eq!(if_stmt.branches.len(), 2);
}

#[test]
fn test_parse_while_loop() {
    let source = "while x > 0 do x = x - 1 end";
    let chunk = parse(source);
    assert!(matches!(&*chunk.statements[0], Stmt::While(_)));
}

#[test]
fn test_parse_repeat_loop() {
    let source = "repeat x = x + 1 until x > 10";
    let chunk = parse(source);
    assert!(matches!(&*chunk.statements[0], Stmt::Repeat(_)));
}

#[test]
fn test_parse_numeric_for() {
    let source = "for i = 1, 10 do print(i) end";
    let chunk = parse(source);
    assert!(matches!(&*chunk.statements[0], Stmt::ForNum(_)));
}

#[test]
fn test_parse_numeric_for_with_step() {
    let source = "for i = 1, 10, 2 do print(i) end";
    let chunk = parse(source);
    assert!(matches!(&*chunk.statements[0], Stmt::ForNum(_)));
}

#[test]
fn test_parse_generic_for() {
    let source = "for k, v in pairs(t) do print(k, v) end";
    let chunk = parse(source);
    let for_in = match &*chunk.statements[0] {
        Stmt::ForIn(f) => f,
        _ => panic!(),
    };
    assert_eq!(for_in.vars.len(), 2);
    assert_eq!(for_in.vars, vec!["k", "v"]);
}

#[test]
fn test_parse_generic_for_single_var() {
    let source = "for k in pairs(t) do print(k) end";
    let chunk = parse(source);
    let for_in = match &*chunk.statements[0] {
        Stmt::ForIn(f) => f,
        _ => panic!(),
    };
    assert_eq!(for_in.vars.len(), 1);
    assert_eq!(for_in.vars[0], "k");
}

#[test]
fn test_parse_function_declaration() {
    let source = "function foo(a, b) return a + b end";
    let chunk = parse(source);
    let func = match &*chunk.statements[0] {
        Stmt::Function(f) => f,
        _ => panic!(),
    };
    assert_eq!(func.name, "foo");
    assert_eq!(func.params, vec!["a", "b"]);
    assert!(!func.is_local);
    assert!(!func.is_method);
}

#[test]
fn test_parse_local_function() {
    let source = "local function foo(x) return x * 2 end";
    let chunk = parse(source);
    let func = match &*chunk.statements[0] {
        Stmt::Function(f) => f,
        _ => panic!(),
    };
    assert_eq!(func.name, "foo");
    assert!(func.is_local);
}

#[test]
fn test_parse_method_definition() {
    let source = "function t:method(x) return self.x + x end";
    let chunk = parse(source);
    let func = match &*chunk.statements[0] {
        Stmt::Function(f) => f,
        _ => panic!(),
    };
    assert_eq!(func.name, "method");
    assert!(func.is_method);
    assert_eq!(func.table_path, vec!["t"]);
    assert_eq!(func.params, vec!["self", "x"]);
}

#[test]
fn test_parse_table_member_function() {
    let source = "function t.a.b.foo() return 42 end";
    let chunk = parse(source);
    let func = match &*chunk.statements[0] {
        Stmt::Function(f) => f,
        _ => panic!(),
    };
    assert_eq!(func.name, "foo");
    assert_eq!(func.table_path, vec!["t", "a", "b"]);
}

#[test]
fn test_parse_vararg_function() {
    let source = "function f(...) return ... end";
    let chunk = parse(source);
    let func = match &*chunk.statements[0] {
        Stmt::Function(f) => f,
        _ => panic!(),
    };
    assert!(func.is_vararg);
}

#[test]
fn test_parse_break() {
    let chunk = parse("while true do break end");
    let while_stmt = match &*chunk.statements[0] {
        Stmt::While(w) => w,
        _ => panic!(),
    };
    assert_eq!(while_stmt.body.len(), 1);
    assert!(matches!(&*while_stmt.body[0], Stmt::Break(_)));
}

#[test]
fn test_parse_do_block() {
    let source = "do local x = 1 end";
    let chunk = parse(source);
    assert!(matches!(&*chunk.statements[0], Stmt::Do(_)));
}

// =====================================================================
// Table constructor tests
// =====================================================================

#[test]
fn test_parse_empty_table() {
    let chunk = parse("return {}");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Table(t) if t.fields.is_empty()));
}

#[test]
fn test_parse_array_table() {
    let chunk = parse("return {1, 2, 3}");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    if let Expr::Table(t) = &*ret.values[0] {
        assert_eq!(t.fields.len(), 3);
        assert!(t.fields.iter().all(|f| f.key.is_none()));
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_keyed_table() {
    let chunk = parse("return {a = 1, b = 2}");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    if let Expr::Table(t) = &*ret.values[0] {
        assert_eq!(t.fields.len(), 2);
        assert!(t.fields.iter().all(|f| f.key.is_some()));
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_expr_key_table() {
    let chunk = parse("return {[x] = 42}");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    if let Expr::Table(t) = &*ret.values[0] {
        assert_eq!(t.fields.len(), 1);
        assert!(t.fields[0].key.is_some());
    } else {
        panic!("Expected table");
    }
}

#[test]
fn test_parse_trailing_separator() {
    let chunk = parse("return {1, 2,}");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    if let Expr::Table(t) = &*ret.values[0] {
        assert_eq!(t.fields.len(), 2);
    } else {
        panic!("Expected table");
    }
}

// =====================================================================
// Function expression tests
// =====================================================================

#[test]
fn test_parse_function_expression() {
    let chunk = parse("return function(x) return x * 2 end");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Function(_)));
}

// =====================================================================
// Postfix expression tests
// =====================================================================

#[test]
fn test_parse_member_access() {
    let chunk = parse("return t.key");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Member(m) if m.member == "key"));
}

#[test]
fn test_parse_index_access() {
    let chunk = parse("return t['key']");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Index(_)));
}

#[test]
fn test_parse_method_call() {
    let chunk = parse("obj:method()");
    let call_stmt = match &*chunk.statements[0] {
        Stmt::Call(c) => c,
        _ => panic!(),
    };
    assert!(matches!(&*call_stmt.call, Expr::Call(c) if c.is_method_call));
}

#[test]
fn test_parse_string_arg_call() {
    let chunk = parse("print 'hello'");
    assert!(matches!(&*chunk.statements[0], Stmt::Call(_)));
}

#[test]
fn test_parse_table_arg_call() {
    let chunk = parse("print{1, 2, 3}");
    assert!(matches!(&*chunk.statements[0], Stmt::Call(_)));
}

// =====================================================================
// Parenthesized expression test
// =====================================================================

#[test]
fn test_parse_paren_expr() {
    let chunk = parse("return (42)");
    let ret = match &*chunk.statements[0] {
        Stmt::Return(r) => r,
        _ => panic!(),
    };
    assert!(matches!(&*ret.values[0], Expr::Paren(_)));
}

// =====================================================================
// Complete program tests
// =====================================================================

#[test]
fn test_parse_fibonacci() {
    let source = r#"
function fib(n)
    if n <= 1 then
        return n
    end
    return fib(n - 1) + fib(n - 2)
end
"#;
    let chunk = parse(source);
    assert_eq!(chunk.statements.len(), 1);
    assert!(matches!(&*chunk.statements[0], Stmt::Function(_)));
}

#[test]
fn test_parse_empty_program() {
    let chunk = parse("");
    assert!(chunk.statements.is_empty());
}

// =====================================================================
// Error tests
// =====================================================================

#[test]
fn test_parse_error_unclosed_string() {
    let err = parse_error("\"hello");
    assert!(!err.message.is_empty());
}

#[test]
fn test_parse_error_unexpected_eof() {
    let err = parse_error("if true then");
    assert!(!err.message.is_empty());
}

#[test]
fn test_parse_error_unclosed_paren() {
    let err = parse_error("print(1, 2");
    assert!(!err.message.is_empty());
}
