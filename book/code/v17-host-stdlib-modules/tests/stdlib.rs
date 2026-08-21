use lua_app::{execute, execute_multi, AppError};
use lua_core::Value;
use lua_vm::VmErrorKind;
use std::process::Command;

fn value(source: &str) -> Value {
    execute(source.as_bytes()).unwrap()
}

#[test]
fn base_library_success_boundary_and_error_contracts() {
    assert_eq!(
        value(
            "assert(_VERSION == 'Lua 5.1')\n\
             shared = 41\n\
             assert(_G.shared == 41)\n\
             _G.shared = 42\n\
             assert(shared == 42)\n\
             assert(type(nil) == 'nil' and type(true) == 'boolean')\n\
             assert(type(1) == 'number' and type('x') == 'string')\n\
             assert(type({}) == 'table' and type(function() end) == 'function')\n\
             assert(tonumber(' 12.5 ') == 12.5 and tonumber('ff', 16) == 255)\n\
             assert(tonumber('not-a-number') == nil and tonumber({}, 10) == nil)\n\
             assert(rawequal(1, 1) and not rawequal({}, {}))\n\
             local a, b = select(2, 10, 20, 30)\n\
             local count = select('#', 10, nil, 30)\n\
             local tail = select(-1, 10, 20, 30)\n\
             assert(a == 20 and b == 30 and count == 3 and tail == 30)\n\
             local x, y, z = unpack({ 7, nil, 9 }, 1, 3)\n\
             assert(x == 7 and y == nil and z == 9)\n\
             local ok, answer = pcall(function(v) return v + 1 end, 41)\n\
             local bad, message = pcall(function() error('boom') end)\n\
             local xok, handled = xpcall(function() error('x') end, function(e) return type(e) end)\n\
             assert(ok and answer == 42 and not bad and type(message) == 'string')\n\
             assert(not xok and handled == 'string')\n\
             assert(tostring(12) == '12')\n\
             assert(collectgarbage('count') > 0)\n\
             collectgarbage('stop')\n\
             collectgarbage('restart')\n\
             collectgarbage('collect')\n\
             print()\n\
             return true"
        ),
        Value::Boolean(true)
    );

    for source in [
        "return assert(false, 'expected')",
        "return select(0, 1)",
        "return unpack(true)",
        "return tonumber('1', 1)",
        "return collectgarbage('unknown')",
    ] {
        let error = execute(source.as_bytes()).unwrap_err();
        assert!(matches!(error, AppError::Runtime(_)));
    }
}

#[test]
fn math_library_covers_every_public_function_and_coercion_error() {
    assert_eq!(
        value(
            "local unary = {\n\
               math.abs, math.acos, math.asin, math.atan, math.ceil, math.cos, math.cosh,\n\
               math.exp, math.floor, math.log, math.log10, math.sin, math.sinh, math.sqrt,\n\
               math.tan, math.tanh, math.deg, math.rad\n\
             }\n\
             for i = 1, #unary do\n\
               assert(type(unary[i]('0.5')) == 'number')\n\
               local ok = pcall(unary[i], {})\n\
               assert(not ok)\n\
             end\n\
             assert(math.atan2(0, -1) > 3)\n\
             assert(math.fmod(-7, 3) == -1)\n\
             local mantissa, exponent = math.frexp(8)\n\
             assert(mantissa == 0.5 and exponent == 4 and math.ldexp(mantissa, exponent) == 8)\n\
             assert(math.max(1, 9, 3) == 9 and math.min(1, 9, 3) == 1)\n\
             local integer, fraction = math.modf(-3.25)\n\
             assert(integer == -3 and fraction == -0.25)\n\
             assert(math.pow(2, 5) == 32 and math.pi > 3.14 and math.huge > 1e300)\n\
             math.randomseed(1234)\n\
             local unit = math.random()\n\
             local die = math.random(6)\n\
             local range = math.random(10, 20)\n\
             assert(unit >= 0 and unit < 1 and die >= 1 and die <= 6)\n\
             assert(range >= 10 and range <= 20)\n\
             local ok1 = pcall(math.random, 0)\n\
             local ok2 = pcall(math.random, 5, 4)\n\
             local ok3 = pcall(math.max)\n\
             return not ok1 and not ok2 and not ok3"
        ),
        Value::Boolean(true)
    );
}

#[test]
fn string_library_preserves_bytes_and_tests_each_exposed_function() {
    assert_eq!(
        value(
            "local binary = string.char(65, 0, 255, 90)\n\
             local a, zero, ff = string.byte(binary, 1, 3)\n\
             assert(a == 65 and zero == 0 and ff == 255 and string.len(binary) == 4)\n\
             assert(string.sub('abcdef', 2, -2) == 'bcde')\n\
             assert(string.sub('abc', 5, 2) == '')\n\
             assert(string.reverse('abc') == 'cba')\n\
             assert(string.rep('ab', 3) == 'ababab' and string.rep('x', 0) == '')\n\
             assert(string.lower('Az') == 'az' and string.upper('Az') == 'AZ')\n\
             local first, last = string.find('a.b.c', '.b', 1, true)\n\
             assert(first == 2 and last == 3)\n\
             assert(string.find('abc', 'z') == nil)\n\
             assert(string.format('%s:%d:%f:%%', 'x', 7, 1.5) == 'x:7:1.5:%')\n\
             assert(('hello'):sub(2, 4) == 'ell')\n\
             local bad_char = pcall(string.char, 256)\n\
             local bad_len = pcall(string.len, {})\n\
             local bad_format = pcall(string.format, '%x', 1)\n\
             return not bad_char and not bad_len and not bad_format"
        ),
        Value::Boolean(true)
    );
}

#[test]
fn table_library_mutates_arrays_and_checks_comparators() {
    assert_eq!(
        value(
            "local items = { 3, 1, 2 }\n\
             table.insert(items, 2, 9)\n\
             assert(table.concat(items, ',', 1, 4) == '3,9,1,2')\n\
             local removed = table.remove(items, 2)\n\
             assert(removed == 9 and #items == 3)\n\
             table.sort(items)\n\
             assert(table.concat(items, '') == '123')\n\
             table.sort(items, function(a, b) return a > b end)\n\
             assert(table.concat(items, '') == '321')\n\
             local sparse = { [2] = true, [9] = true, name = true }\n\
             assert(table.maxn(sparse) == 9)\n\
             assert(table.remove({}, 1) == nil)\n\
             local wrong_table = pcall(table.concat, true)\n\
             local wrong_insert = pcall(table.insert, items, 0, 1)\n\
             local wrong_sort = pcall(table.sort, { {}, {} })\n\
             return not wrong_table and not wrong_insert and not wrong_sort"
        ),
        Value::Boolean(true)
    );
}

#[test]
fn coroutine_library_remains_registered_through_stdlib_catalog() {
    assert_eq!(
        execute_multi(
            b"local co = coroutine.create(function(v) return coroutine.yield(v + 1) end)\n\
              local ok, yielded = coroutine.resume(co, 41)\n\
              return ok, yielded, coroutine.status(co)"
        )
        .unwrap(),
        vec![
            Value::Boolean(true),
            Value::Number(42.0),
            Value::String(lua_core::ByteString::new(&b"suspended"[..])),
        ]
    );
}

#[test]
fn standard_library_argument_errors_are_classified_as_type_errors() {
    let error = match execute(b"return math.sqrt({})").unwrap_err() {
        AppError::Runtime(error) => error,
        other => panic!("expected runtime error, got {other}"),
    };
    assert_eq!(error.kind, VmErrorKind::Type);
    assert!(error.message.contains("bad argument #1"));
}

#[test]
fn representative_core_library_result_matches_lua_5_1_when_available() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/stdlib_diff.lua");
    let source = std::fs::read(fixture).unwrap();
    let ours = execute(&source).unwrap();

    let oracle = match Command::new("lua").arg(fixture).output() {
        Ok(output) if output.status.success() => output,
        Ok(output) => panic!(
            "Lua oracle failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("failed to start Lua oracle: {error}"),
    };
    let oracle_number = String::from_utf8_lossy(&oracle.stdout)
        .trim()
        .parse::<f64>()
        .unwrap();
    assert_eq!(ours, Value::Number(oracle_number));
}
