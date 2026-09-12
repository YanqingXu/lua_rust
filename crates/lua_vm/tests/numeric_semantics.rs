use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{OpCode, get_opcode};
use lua_compiler::parser::Parser;
use lua_core::value::Value;
use lua_vm::Runtime;

fn run_numeric(source: &str) -> (f64, Vec<OpCode>) {
    let mut runtime = Runtime::new();
    let (proto, opcodes) = {
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, strings) = parts.split_mut();
        let chunk = Parser::new(source).parse().unwrap();
        let proto = CodeGenerator::new_with_pool(gc, strings)
            .generate(&chunk, "@numeric-semantics")
            .unwrap();
        let opcodes = proto.code().iter().copied().map(get_opcode).collect();
        (gc.create(proto), opcodes)
    };
    runtime.execute_proto(proto).unwrap();
    let value = runtime
        .parts_mut()
        .unwrap()
        .state()
        .stack
        .at(0)
        .cloned()
        .unwrap();
    let Value::Number(number) = value else {
        panic!("expected numeric result from {source}, got {value:?}");
    };
    (number, opcodes)
}

#[test]
fn zero_divisors_and_nan_results_are_evaluated_at_runtime() {
    // Lua 5.1's compiler declines zero-divisor and NaN folds. The resulting
    // opcodes still produce numbers; they must not become runtime errors.
    for (source, opcode, expected) in [
        ("return 1 / 0", OpCode::DIV, f64::INFINITY),
        ("return -1 / 0", OpCode::DIV, f64::NEG_INFINITY),
        ("return 1 / -0", OpCode::DIV, f64::NEG_INFINITY),
        ("return -1 / -0", OpCode::DIV, f64::INFINITY),
        ("return 0 / 0", OpCode::DIV, f64::NAN),
        ("return 1 % 0", OpCode::MOD, f64::NAN),
        ("return -1 % -0", OpCode::MOD, f64::NAN),
        ("return (-1)^0.5", OpCode::POW, f64::NAN),
    ] {
        let (actual, opcodes) = run_numeric(source);
        assert!(opcodes.contains(&opcode), "{source}");
        if expected.is_nan() {
            assert!(actual.is_nan(), "{source}: {actual:?}");
        } else {
            assert_eq!(actual.to_bits(), expected.to_bits(), "{source}");
        }
    }
}

#[test]
fn folded_results_match_register_arithmetic_including_signed_zero_and_infinity() {
    for (lhs, operator, rhs, opcode, expected) in [
        ("1", "/", "2", OpCode::DIV, 0.5_f64),
        ("1e308", "/", "1e-308", OpCode::DIV, f64::INFINITY),
        ("-0", "/", "1", OpCode::DIV, -0.0),
        ("1", "%", "-2", OpCode::MOD, -1.0),
        ("-1", "%", "2", OpCode::MOD, 1.0),
        ("0", "^", "0", OpCode::POW, 1.0),
        ("-0", "^", "-3", OpCode::POW, f64::NEG_INFINITY),
        ("-0", "^", "2", OpCode::POW, 0.0),
    ] {
        let folded = format!("return ({lhs}) {operator} ({rhs})");
        let dynamic = format!("local a={lhs}; local b={rhs}; return a {operator} b");
        let (constant, constant_ops) = run_numeric(&folded);
        let (register, register_ops) = run_numeric(&dynamic);
        assert!(!constant_ops.contains(&opcode), "{folded}");
        assert!(register_ops.contains(&opcode), "{dynamic}");
        assert_eq!(constant.to_bits(), expected.to_bits(), "{folded}");
        assert_eq!(register.to_bits(), expected.to_bits(), "{dynamic}");
    }
}
