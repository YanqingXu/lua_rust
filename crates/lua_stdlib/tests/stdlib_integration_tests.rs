use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_stdlib::catalog::open_all;
use lua_vm::execute::execute_proto;
use lua_vm::state::LuaState;
use std::path::{Path, PathBuf};

fn compile_and_run(source: &str) -> (LuaState, GarbageCollector) {
    let mut gc = GarbageCollector::new();
    let global_table = gc.create_root(Table::new());
    let mut state = LuaState::with_global_table(global_table);

    open_all(&mut state, &mut gc);

    let mut parser = Parser::new(source);
    let chunk = parser.parse().expect("parse should succeed");
    let cg = CodeGenerator::new(&mut gc);
    let proto = cg
        .generate(&chunk, "<stdlib-test>")
        .expect("codegen should succeed");

    execute_proto(&mut state, &proto, &mut gc).expect("VM should execute");
    (state, gc)
}

fn return_value(state: &LuaState) -> Value {
    state.stack.at(0).cloned().unwrap_or(Value::Nil)
}

fn returned_string(state: &LuaState) -> String {
    match return_value(state) {
        Value::String(s) => {
            // SAFETY: the returned string is still owned by the live test GC.
            unsafe { s.as_ref() }
                .expect("string ref should be valid")
                .data()
                .to_string()
        }
        value => panic!("expected string, got {value:?}"),
    }
}

fn write_temp_lua_file(name: &str, source: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    path.push(format!(
        "lua_rust_{name}_{}_{}.lua",
        std::process::id(),
        stamp
    ));
    std::fs::write(&path, source).expect("test lua file should be writable");
    path
}

fn write_temp_lua_bytes(name: &str, source: &[u8]) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    path.push(format!(
        "lua_rust_{name}_{}_{}.lua",
        std::process::id(),
        stamp
    ));
    std::fs::write(&path, source).expect("test lua file should be writable");
    path
}

fn lua_path_literal(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    format!("\"{}\"", path.replace('"', "\\\""))
}

fn alien_signals_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("lua")
        .join("alien_signals");
    assert!(
        dir.is_dir(),
        "alien_signals Lua tests should exist at {}",
        dir.display()
    );
    dir
}

#[test]
fn math_multi_arg_functions_return_results() {
    let (state, _gc) = compile_and_run("return math.pow(2, 8)");
    assert_eq!(return_value(&state), Value::Number(256.0));

    let (state, _gc) = compile_and_run("return math.min(1, 5, 3, 9, 2)");
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run("return math.max(1, 5, 3, 9, 2)");
    assert_eq!(return_value(&state), Value::Number(9.0));
}

#[test]
fn base_collectgarbage_and_global_self_are_available() {
    let (state, _gc) = compile_and_run(
        "local before = collectgarbage('count'); local after = collectgarbage(); if _G and rawget(_G, 'math') == math and type(before) == 'number' and type(after) == 'number' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn math_random_and_os_clock_support_sort_scripts() {
    let (state, _gc) = compile_and_run(
        "math.randomseed(123); local a = math.random(); local b = math.random(10); local c = math.random(5, 8); local t = os.clock(); if a >= 0 and a < 1 and b >= 1 and b <= 10 and c >= 5 and c <= 8 and type(t) == 'number' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn math_modf_constants_and_string_arguments_work() {
    let (state, _gc) = compile_and_run(
        "local a, b = math.modf(' 3.5 '); local v, e = math.frexp(math.pi); if a == 3 and b == 0.5 and math.mod(10, 3) == 1 and math.abs(math.ldexp(v, e) - math.pi) < 1e-10 and math.huge > 10e30 and math.abs(math.pi - math.rad(180)) < 1e-10 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn io_tmpfile_supports_memory_file_workflow() {
    let (state, _gc) = compile_and_run(
        "local f = assert(io.tmpfile()); f:write('return ', 20, '+', 22); f:seek('set', 0); local chunk = f:read('*a'); local ok = f:close(); return assert(loadstring(chunk))() + (ok and 1 or 0)",
    );
    assert_eq!(return_value(&state), Value::Number(43.0));
}

#[test]
fn io_file_handles_lines_and_os_file_helpers_work() {
    let path = write_temp_lua_file("io_file", "");
    let mut other = path.clone();
    other.set_extension("renamed");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&other);
    let source = format!(
        "local file = {file}; local other = {other}; os.remove(file); os.remove(other); local f = assert(io.open(file, 'w')); assert(type(f) == 'userdata' and io.type(f) == 'file'); assert(f:setvbuf('no')); assert(f:write('alpha\\n', 12, '\\n', ' 3.5\\n')); assert(f:close()); assert(os.rename(file, other)); local input = assert(io.input(other)); assert(io.input() == input); local a, b, c = io.read('*l', '*n', '*n'); assert(a == 'alpha' and b == 12 and c == 3.5); input:seek('set'); local collected = ''; for line in input:lines() do collected = collected .. line .. '|' end; assert(collected == 'alpha|12| 3.5|'); input:close(); assert(tostring(input) == 'file (closed)' and io.type(input) == 'closed file'); assert(os.remove(other)); return 1",
        file = lua_path_literal(&path),
        other = lua_path_literal(&other),
    );
    let (state, _gc) = compile_and_run(&source);
    assert_eq!(return_value(&state), Value::Number(1.0));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&other);
}

#[test]
fn os_setlocale_supports_c_locale_queries() {
    let (state, _gc) = compile_and_run(
        "if os.setlocale() == 'C' and os.setlocale('C') == 'C' and os.setlocale(nil, 'numeric') == 'C' and os.setlocale('pt_BR') == nil then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn os_time_date_and_difftime_are_self_consistent() {
    let (state, _gc) = compile_and_run(
        "local t = os.time{year = 2000, month = 10, day = 1, hour = 23, min = 12, sec = 17}; local T = os.date('!*t', t); local s = os.date('!%Y-%m-%d %H:%M:%S %w %j', t); local t2 = os.time{year = 2000, month = 10, day = 1, hour = 23, min = 10, sec = 19}; if T.year == 2000 and T.month == 10 and T.day == 1 and T.hour == 23 and T.min == 12 and T.sec == 17 and T.wday == 1 and T.yday == 275 and T.isdst == false and s == '2000-10-01 23:12:17 0 275' and os.difftime(t, t2) == 118 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn c_functions_can_be_called_from_locals() {
    let (state, _gc) = compile_and_run("local pow = math.pow; return pow(3, 4)");
    assert_eq!(return_value(&state), Value::Number(81.0));
}

#[test]
fn lua_functions_compile_to_real_sub_protos() {
    let (state, _gc) = compile_and_run("function add(a, b) return a + b end; return add(2, 3)");
    assert_eq!(return_value(&state), Value::Number(5.0));

    let (state, _gc) =
        compile_and_run("local function twice(x) return x * 2 end; return twice(21)");
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run("local f = function(x) return x + 7 end; return f(5)");
    assert_eq!(return_value(&state), Value::Number(12.0));
}

#[test]
fn tail_calls_to_c_functions_return_results() {
    let (state, _gc) = compile_and_run(
        "local function kind(x) return type(x) end; if kind({}) == 'table' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn nested_and_recursive_lua_calls_keep_their_frames() {
    let (state, _gc) = compile_and_run(
        "function add(a, b) return a + b end; function use(x) return add(x, 3) end; return use(4)",
    );
    assert_eq!(return_value(&state), Value::Number(7.0));

    let (state, _gc) = compile_and_run(
        "function fact(n) if n <= 1 then return 1 end return n * fact(n - 1) end; return fact(5)",
    );
    assert_eq!(return_value(&state), Value::Number(120.0));
}

#[test]
fn method_calls_pass_the_receiver_as_self() {
    let (state, _gc) = compile_and_run(
        "local t = {value = 10, add = function(self, x) return self.value + x end}; return t:add(5)",
    );
    assert_eq!(return_value(&state), Value::Number(15.0));
}

#[test]
fn table_member_function_definitions_are_assigned() {
    let (state, _gc) = compile_and_run(
        "local ns = {inner = {}}; function ns.inner.mul(a, b) return a * b end; return ns.inner.mul(3, 4)",
    );
    assert_eq!(return_value(&state), Value::Number(12.0));

    let (state, _gc) = compile_and_run(
        "local t = {value = 9}; function t:add(x) return self.value + x end; return t:add(6)",
    );
    assert_eq!(return_value(&state), Value::Number(15.0));
}

#[test]
fn table_field_assignment_updates_existing_string_keys() {
    let (state, _gc) = compile_and_run("local t = {x = 1}; t.x = 5; return t.x");
    assert_eq!(return_value(&state), Value::Number(5.0));

    let (state, _gc) = compile_and_run(
        "local t = {x = 1}; function t:set(v) self.x = v; return self.x end; return t:set(5) + t.x",
    );
    assert_eq!(return_value(&state), Value::Number(10.0));
}

#[test]
fn closures_capture_and_close_upvalues() {
    let (state, _gc) = compile_and_run(
        "local function make(x) return function(y) return x + y end end; local f = make(10); return f(5)",
    );
    assert_eq!(return_value(&state), Value::Number(15.0));

    let (state, _gc) = compile_and_run(
        "local function counter() local n = 0; return function() n = n + 1; return n end end; local c = counter(); local a = c(); local b = c(); local d = c(); return a + b * 10 + d * 100",
    );
    assert_eq!(return_value(&state), Value::Number(321.0));
}

#[test]
fn nested_and_shared_closures_use_the_same_upvalue() {
    let (state, _gc) = compile_and_run(
        "local function outer() local x = 3; local function mid() return function(y) return x + y end end; return mid() end; local f = outer(); return f(4)",
    );
    assert_eq!(return_value(&state), Value::Number(7.0));

    let (state, _gc) = compile_and_run(
        "local function pair() local n = 0; return function() n = n + 1; return n end, function() n = n + 10; return n end end; local a, b = pair(); local x = a(); local y = b(); local z = a(); return x + y * 10 + z * 100",
    );
    assert_eq!(return_value(&state), Value::Number(1311.0));
}

#[test]
fn call_arguments_use_stable_registers_for_nested_calls() {
    let (state, _gc) = compile_and_run(
        "local function add(a, b) return a + b end; local function id(x) return x end; return id(add(2, 3))",
    );
    assert_eq!(return_value(&state), Value::Number(5.0));
}

#[test]
fn vararg_locals_and_return_expand_multiple_values() {
    let (state, _gc) = compile_and_run(
        "local function collect(...) local a, b, c = ...; return a * 100 + b * 10 + c end; return collect(1, 2, 3)",
    );
    assert_eq!(return_value(&state), Value::Number(123.0));

    let (state, _gc) = compile_and_run(
        "local function pass(...) return ... end; local a, b, c = pass(4, 5, 6); return a * 100 + b * 10 + c",
    );
    assert_eq!(return_value(&state), Value::Number(456.0));
}

#[test]
fn vararg_functions_create_legacy_arg_table_when_needed() {
    let (state, _gc) = compile_and_run(
        "function f(a, ...) if type(arg) == 'table' and arg.n == 3 and arg[1] == 1 and arg[2] == nil and arg[3] == 3 then return 1 end return 0 end; return f({}, 1, nil, 3)",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "function g(...) local x = {...}; if arg == nil and x[1] == 4 and x[2] == 5 then return 1 end return 0 end; return g(4, 5)",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local ok, n = pcall(function(...) return arg.n end, 'a', 'b'); if ok and n == 2 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn return_call_preserves_multiple_results_for_wrappers() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local function pass() return vals() end; local a, b, c = pass(); return a * 100 + b * 10 + c",
    );
    assert_eq!(return_value(&state), Value::Number(123.0));
}

#[test]
fn final_call_argument_expands_multiple_results() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local function pack(a, b, c, d) return a * 1000 + b * 100 + c * 10 + d end; return pack(9, vals())",
    );
    assert_eq!(return_value(&state), Value::Number(9123.0));

    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local function sum(a, b, c) return a + b + c end; return sum(vals())",
    );
    assert_eq!(return_value(&state), Value::Number(6.0));
}

#[test]
fn nested_call_and_vararg_pipeline_preserves_multiple_results() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 7, 8, 9 end; local function pass(...) return ... end; local a, b, c = pass(vals()); return a * 100 + b * 10 + c",
    );
    assert_eq!(return_value(&state), Value::Number(789.0));
}

#[test]
fn call_statement_temporaries_do_not_shift_later_locals() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local function wrap() return vals() end; local a, b, c = wrap(); print('touch', a, b, c); local function sum(a, b, c) return a + b + c end; return sum(vals())",
    );
    assert_eq!(return_value(&state), Value::Number(6.0));
}

#[test]
fn non_final_multi_return_expression_still_collapses_to_one_value() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local a, b = vals(), 9; return a * 10 + b",
    );
    assert_eq!(return_value(&state), Value::Number(19.0));
}

#[test]
fn assignment_expands_final_call_and_vararg_results() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local a, b, c = 0, 0, 0; a, b, c = vals(); return a * 100 + b * 10 + c",
    );
    assert_eq!(return_value(&state), Value::Number(123.0));

    let (state, _gc) = compile_and_run(
        "local function fill(...) local a, b, c = 0, 0, 0; a, b, c = ...; return a * 100 + b * 10 + c end; return fill(4, 5, 6)",
    );
    assert_eq!(return_value(&state), Value::Number(456.0));

    let (state, _gc) = compile_and_run(
        "local function vals() return 7, 8 end; local a, b, c = 0, 0, 9; a, b, c = vals(); if c == nil then return a * 10 + b end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(78.0));

    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local a, b = 0, 0; a, b = vals(), 9; return a * 10 + b",
    );
    assert_eq!(return_value(&state), Value::Number(19.0));
}

#[test]
fn table_constructor_expands_only_final_multi_return_field() {
    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local t = {vals()}; return t[1] * 100 + t[2] * 10 + t[3]",
    );
    assert_eq!(return_value(&state), Value::Number(123.0));

    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local t = {0, vals()}; return t[1] * 1000 + t[2] * 100 + t[3] * 10 + t[4]",
    );
    assert_eq!(return_value(&state), Value::Number(123.0));

    let (state, _gc) = compile_and_run(
        "local function vals() return 1, 2, 3 end; local t = {vals(), 9}; return t[1] * 10 + t[2]",
    );
    assert_eq!(return_value(&state), Value::Number(19.0));

    let (state, _gc) = compile_and_run(
        "local function pack(...) local t = {...}; return t[1] * 100 + t[2] * 10 + t[3] end; return pack(4, 5, 6)",
    );
    assert_eq!(return_value(&state), Value::Number(456.0));
}

#[test]
fn base_type_returns_a_string_value() {
    let (state, _gc) = compile_and_run("return type(math)");
    assert_eq!(returned_string(&state), "table");

    let (state, _gc) = compile_and_run("return type(print)");
    assert_eq!(returned_string(&state), "function");
}

#[test]
fn base_tostring_returns_a_string_value() {
    let (state, _gc) = compile_and_run("return tostring(42)");
    assert_eq!(returned_string(&state), "42");

    let (state, _gc) = compile_and_run("return tostring(false)");
    assert_eq!(returned_string(&state), "false");
}

#[test]
fn base_assert_select_tonumber_and_unpack_work() {
    let (state, _gc) =
        compile_and_run("local ok, value = assert(true, 42); if ok then return value end return 0");
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run(
        "local function f(...) local n = select('#', ...); local a, b = select(2, ...); return n * 100 + a * 10 + b end; return f(4, 5, 6)",
    );
    assert_eq!(return_value(&state), Value::Number(356.0));

    let (state, _gc) = compile_and_run("return tonumber('101', 2) + tonumber('3.5')");
    assert_eq!(return_value(&state), Value::Number(8.5));

    let (state, _gc) =
        compile_and_run("local a, b, c = unpack({7, 8, 9}); return a * 100 + b * 10 + c");
    assert_eq!(return_value(&state), Value::Number(789.0));
}

#[test]
fn base_loadstring_compiles_and_returns_executable_functions() {
    let (state, _gc) = compile_and_run("local f = assert(loadstring('return 42')); return f()");
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run(
        "local src = 'local function add(a, b) return a + b end; return add(20, 22)'; local f = assert(loadstring(src)); return f()",
    );
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run(
        "local f = assert(loadstring('loaded_marker = 37; return loaded_marker + 5')); return f()",
    );
    assert_eq!(return_value(&state), Value::Number(42.0));
}

#[test]
fn base_loadstring_reports_compile_errors_as_nil_and_message() {
    let (state, _gc) = compile_and_run(
        "local f, msg = loadstring('return return'); if f == nil and type(msg) == 'string' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local f, msg = loadstring(123); if f == nil and string.match(msg, 'string expected') then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn base_load_reads_source_from_reader_functions() {
    let (state, _gc) = compile_and_run(
        "local src = 'return 40 + 2'; local i = 0; local f = assert(load(function() i = i + 1; return string.sub(src, i, i) end, 'reader')); return f()",
    );
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run(
        "local f, msg = load(function() error('reader boom') end); if f == nil and string.match(msg, 'reader boom') then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local i = 0; local f, msg = load(function() i = i + 1; return string.sub('*a = 1', i, i) end); if f == nil and type(msg) == 'string' then return i end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(2.0));
}

#[test]
fn debug_getinfo_reports_lua_function_source() {
    let (state, _gc) = compile_and_run(
        "local done = false; local f = assert(load(function() if done then return nil end; done = true; return 'return 42' end, 'modname')); if debug.getinfo(f).source == 'modname' then return f() end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run(
        "local f = assert(loadstring('local x = 1\\nreturn debug.getinfo(1).currentline')); return f()",
    );
    assert_eq!(return_value(&state), Value::Number(2.0));
}

#[test]
fn debug_getinfo_reports_global_function_name() {
    let (state, _gc) = compile_and_run(
        "function F(a) return debug.getinfo(1, 'n').name end; return F(1) == 'F' and 1 or 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn coroutine_wrap_yields_recursive_generator_values() {
    let (state, _gc) = compile_and_run(
        "local function gen(c, n) if n == 0 then coroutine.yield(c) else gen(c .. 'a', n - 1); gen(c .. 'b', n - 1) end end local out = '' for s in coroutine.wrap(function() gen('', 2) end) do out = out .. s .. ',' end return out",
    );
    assert_eq!(returned_string(&state), "aa,ab,ba,bb,");
}

#[test]
fn string_dump_round_trips_lua_functions_inside_this_vm() {
    let (state, _gc) = compile_and_run(
        "local dumped = string.dump(loadstring('x = 1; return x')); local i = 0; local f = assert(load(function() i = i + 1; return string.sub(dumped, i, i) end)); return f()",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn debug_setupvalue_updates_dumped_function_upvalues() {
    let (state, _gc) = compile_and_run(
        "local a, b = 20, 30; local x = loadstring(string.dump(function(x) if x == 'set' then a = 10 + b; b = b + 1 else return a end end)); if x() ~= nil then return 0 end; if debug.setupvalue(x, 1, 'hi') ~= 'a' then return 0 end; if x() ~= 'hi' then return 0 end; if debug.setupvalue(x, 2, 13) ~= 'b' then return 0 end; if debug.setupvalue(x, 3, 10) ~= nil then return 0 end; x('set'); if x() ~= 23 then return 0 end; x('set'); return x()",
    );
    assert_eq!(return_value(&state), Value::Number(24.0));
}

#[test]
fn base_loadfile_and_dofile_compile_and_execute_files() {
    let load_path = write_temp_lua_file(
        "loadfile",
        "file_marker = 21; return file_marker + 21, 'loaded'",
    );
    let load_lit = lua_path_literal(&load_path);
    let (state, _gc) = compile_and_run(&format!(
        "local f = assert(loadfile({load_lit})); local a, b = f(); if b == 'loaded' then return a end return 0"
    ));
    assert_eq!(return_value(&state), Value::Number(42.0));
    let _ = std::fs::remove_file(load_path);

    let do_path = write_temp_lua_file("dofile", "return 7, 8");
    let do_lit = lua_path_literal(&do_path);
    let (state, _gc) =
        compile_and_run(&format!("local a, b = dofile({do_lit}); return a * 10 + b"));
    assert_eq!(return_value(&state), Value::Number(78.0));
    let _ = std::fs::remove_file(do_path);
}

#[test]
fn base_dofile_preserves_non_utf8_source_bytes() {
    let mut source = b"return string.byte(\"".to_vec();
    source.push(0xe1);
    source.extend_from_slice(b"\")");
    let path = write_temp_lua_bytes("latin1_source", &source);
    let lit = lua_path_literal(&path);

    let (state, _gc) = compile_and_run(&format!("return dofile({lit})"));
    assert_eq!(return_value(&state), Value::Number(225.0));
    let _ = std::fs::remove_file(path);
}

#[test]
fn base_loadfile_and_dofile_report_file_errors() {
    let missing_path = std::env::temp_dir().join(format!(
        "lua_rust_missing_{}_{}.lua",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    let missing_lit = lua_path_literal(&missing_path);

    let (state, _gc) = compile_and_run(&format!(
        "local f, msg = loadfile({missing_lit}); if f == nil and type(msg) == 'string' then return 1 end return 0"
    ));
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(&format!(
        "local ok, msg = pcall(dofile, {missing_lit}); if not ok and type(msg) == 'string' then return 1 end return 0"
    ));
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn package_require_returns_builtin_libraries() {
    let (state, _gc) = compile_and_run(
        "if require('math') == math and require('table') == table and package.loaded.math == math then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn package_require_loads_lua_files_and_caches_results() {
    let module_path = write_temp_lua_file(
        "require_module",
        "load_count = (load_count or 0) + 1; return { value = load_count }",
    );
    let module_lit = lua_path_literal(&module_path);
    let (state, _gc) = compile_and_run(&format!(
        "package.path = {module_lit}; local a = require('sample'); local b = require('sample'); if a == b then return a.value * 10 + b.value end return 0"
    ));
    assert_eq!(return_value(&state), Value::Number(11.0));

    std::fs::remove_file(module_path).ok();
}

#[test]
fn package_require_honors_package_loaded_written_by_module() {
    let module_path = write_temp_lua_file(
        "require_loaded",
        "package.loaded[...] = 25; return require(...)",
    );
    let module_lit = lua_path_literal(&module_path);
    let (state, _gc) = compile_and_run(&format!(
        "package.path = {module_lit}; return require('loaded_by_module')"
    ));
    assert_eq!(return_value(&state), Value::Number(25.0));

    std::fs::remove_file(module_path).ok();
}

#[test]
fn package_module_preload_and_io_output_support_require_workflows() {
    let module_path = write_temp_lua_file("io_output_module", "");
    let module_lit = lua_path_literal(&module_path);
    let (state, _gc) = compile_and_run(&format!(
        "io.output({module_lit}); io.write('return {{value = 42}}'); assert(io.close(io.output())); package.path = {module_lit}; local m = require('generated'); if m.value ~= 42 then return 0 end local p = package; package = {{}}; p.preload.pl = function (...) module(..., p.seeall); function xuxu(x) return x + 20 end end; require('pl'); package = p; if require('pl') == pl and pl.xuxu(10) == 30 and pl._G == _G then return 1 end return 0"
    ));
    assert_eq!(return_value(&state), Value::Number(1.0));

    std::fs::remove_file(module_path).ok();
}

#[test]
fn multiple_assignment_freezes_indexed_lvalue_addresses() {
    let (state, _gc) = compile_and_run(
        "local a,i,j,b; a = {'a','b'}; i=1; j=2; b=a; i, a[i], a, j, a[j], a[i+j] = j, i, i, b, j, i; if i == 2 and b[1] == 1 and a == 1 and j == b and b[2] == 2 and b[3] == 1 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn base_pcall_wraps_success_results_and_runtime_errors() {
    let (state, _gc) = compile_and_run(
        "local ok, a, b = pcall(function(x) return x + 1, x + 2 end, 40); if ok then return a * 100 + b end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(4142.0));

    let (state, _gc) = compile_and_run(
        "local ok, msg = pcall(function() error('boom') end); if not ok and string.match(msg, 'boom') then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local payload = {msg = 'x'}; local ok, err = pcall(function() error(payload) end); if not ok and err == payload then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local marker = 0; local ok = pcall(function() marker = 42; error('stop') end); if not ok then return marker end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state, _gc) = compile_and_run(
        "local f = assert(loadstring('return 7, 8')); local ok, a, b = pcall(f); if ok then return a * 10 + b end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(78.0));

    let (state, _gc) = compile_and_run(
        "local t = {}; local nan = 10e500 - 10e400; local ok = pcall(function() t[nan] = 1 end); if not ok and t[nan] == nil then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local t = {}; local ok = pcall(function() t[nil] = 1 end); if not ok then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn official_error_surface_regressions_are_preserved() {
    let (state, _gc) = compile_and_run(
        r#"
        local checks = {
            {function() tostring() end, "tostring"},
            {function() tonumber() end, "tonumber"},
            {function() math.sin() end, "'sin'"},
        }
        for _, check in ipairs(checks) do
            local ok, msg = pcall(check[1])
            if ok or type(msg) ~= "string" or not string.find(msg, check[2], 1, true) then
                return 0
            end
        end

        local ok0, msg0 = pcall(function() error("raw", 0) end)
        if ok0 or msg0 ~= "raw" then return 0 end

        local ok1, msg1 = pcall(function() error("where", 1) end)
        if ok1 or not string.find(msg1, ":%d+: where") then return 0 end

        local payload = {}
        local ok_obj, err_obj = pcall(function() error(payload, 1) end)
        if ok_obj or err_obj ~= payload then return 0 end

        local ok_handler, handler_msg = xpcall(error, error)
        if ok_handler or handler_msg ~= "error in error handling" then return 0 end

        function lineerror(s)
            local ok, msg = pcall(assert(loadstring(s)))
            local line = type(msg) == "string" and string.match(msg, ":(%d+):")
            return ok, line and line + 0, msg
        end

        a = {}
        local ok_line, line, msg = lineerror("function a.x.y ()\nreturn 1\nend")
        if ok_line or line ~= 1 or not string.find(msg, "field 'x'", 1, true) then return 0 end

        return 1
        "#,
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn official_gc_weak_table_and_finalizer_patterns_work() {
    let (state, _gc) = compile_and_run(
        r#"
        local bytes = gcinfo()
        local guard = 0
        repeat
            local nbytes = gcinfo()
            if nbytes < bytes then break end
            bytes = nbytes
            guard = guard + 1
            local a = {}
        until guard > 20
        assert(guard <= 20)

        local function dosteps(siz)
            collectgarbage()
            collectgarbage("stop")
            local a = {}
            for i = 1, 20 do a[i] = {{}} end
            local x = gcinfo()
            local i = 0
            repeat i = i + 1 until collectgarbage("step", siz)
            assert(gcinfo() < x)
            return i
        end
        local d0, d6, d2, d10000 = dosteps(0), dosteps(6), dosteps(2), dosteps(10000)
        assert(d0 > 10 and d6 < d2 and d10000 == 1)

        local lim = 4
        local a = {}; setmetatable(a, {__mode = "v"})
        for i = 1, lim do a[i] = {} end
        for i = 1, lim do local t = {}; a[t] = t end
        for i = 1, lim do a[i + lim] = i .. "x" end
        collectgarbage()
        local count = 0
        for k, v in pairs(a) do
            assert(k == v or k - lim .. "x" == v)
            count = count + 1
        end
        assert(count == 2 * lim)

        collectgarbage("stop")
        local n = 3
        local u = newproxy(true)
        local s = 0
        local t = {[u] = 0}; setmetatable(t, {__mode = "vk"})
        for i = 1, n do t[newproxy(u)] = i end
        local t1 = {}; for k, v in pairs(t) do t1[k] = v end
        for k, v in pairs(t1) do t[v] = k end
        getmetatable(u).t1 = t1
        do
            local u = u
            getmetatable(u).__gc = function(o)
                assert(t[o] == n - s)
                assert(t[n - s] == nil)
                assert(getmetatable(o).t1[o] == n - s)
                s = s + 1
            end
        end
        t1, u = nil
        collectgarbage()
        assert(s == n + 1)
        collectgarbage()
        assert(next(t) == nil)

        return 1
        "#,
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn base_xpcall_invokes_error_handlers() {
    let (state, _gc) = compile_and_run(
        "local ok, a, b = xpcall(function() return 7, 8 end, function(err) return err end); if ok then return a * 10 + b end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(78.0));

    let (state, _gc) = compile_and_run(
        "local ok, msg = xpcall(function() error('boom') end, function(err) return 'handled:' .. type(err) end); if not ok and msg == 'handled:string' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local ok, result = xpcall(function() error({msg = 'x'}) end, function(err) return {msg = err.msg .. 'y'} end); if not ok and type(result) == 'table' and result.msg == 'xy' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local marker = 0; local ok, msg = xpcall(function() marker = 42; error('boom') end, function(err) return err end); if not ok and marker == 42 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn base_raw_table_and_metatable_helpers_work() {
    let (state, _gc) = compile_and_run(
        "local t = {}; rawset(t, 'x', 7); if rawequal(rawget(t, 'x'), 7) then return t.x end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(7.0));

    let (state, _gc) = compile_and_run(
        "local t = {}; local mt = {answer = 42}; setmetatable(t, mt); return getmetatable(t).answer",
    );
    assert_eq!(return_value(&state), Value::Number(42.0));
}

#[test]
fn table_insert_remove_and_getn_work() {
    let (state, _gc) = compile_and_run(
        "local t = {1, 2, 4}; table.insert(t, 3, 3); return t[1] + t[2] + t[3] + t[4] + table.getn(t)",
    );
    assert_eq!(return_value(&state), Value::Number(14.0));

    let (state, _gc) = compile_and_run(
        "local t = {1, 2, 3}; local removed = table.remove(t, 2); return removed * 10 + t[2] + table.getn(t)",
    );
    assert_eq!(return_value(&state), Value::Number(25.0));
}

#[test]
fn table_concat_and_maxn_work() {
    let (state, _gc) = compile_and_run("return table.concat({'a', 'b', 3}, '-')");
    assert_eq!(returned_string(&state), "a-b-3");

    let (state, _gc) = compile_and_run("local t = {[3] = 'x', [10] = 'y'}; return table.maxn(t)");
    assert_eq!(return_value(&state), Value::Number(10.0));
}

#[test]
fn table_foreach_and_foreachi_follow_lua51_callbacks() {
    let (state, _gc) = compile_and_run(
        "local t = {x = 90, y = 8}; return table.foreach(t, function(k, v) if k == 'x' then return v end end)",
    );
    assert_eq!(return_value(&state), Value::Number(90.0));

    let (state, _gc) = compile_and_run("table.foreach({}, error); return 1");
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local n = 1; table.foreachi({n = 3}, function(i, v) if n ~= i or v then return 99 end; n = n + 1 end); return n",
    );
    assert_eq!(return_value(&state), Value::Number(4.0));

    let (state, _gc) = compile_and_run(
        "return table.foreachi({'a', 'b', 'c'}, function(i, v) if i == 2 then return v end end)",
    );
    assert_eq!(returned_string(&state), "b");
}

#[test]
fn table_sort_orders_default_values() {
    let (state, _gc) =
        compile_and_run("local t = {3, 1, 4, 2}; table.sort(t); return table.concat(t, ',')");
    assert_eq!(returned_string(&state), "1,2,3,4");

    let (state, _gc) =
        compile_and_run("local t = {'c', 'a', 'b'}; table.sort(t); return table.concat(t, '')");
    assert_eq!(returned_string(&state), "abc");

    let (state, _gc) = compile_and_run("local t = {}; table.sort(t); return table.getn(t)");
    assert_eq!(return_value(&state), Value::Number(0.0));
}

#[test]
fn table_sort_accepts_lua_comparators() {
    let (state, _gc) = compile_and_run(
        "local t = {3, 1, 4, 2}; table.sort(t, function(a, b) return a > b end); return table.concat(t, ',')",
    );
    assert_eq!(returned_string(&state), "4,3,2,1");

    let (state, _gc) = compile_and_run(
        "local t = {'pear', 'fig', 'banana', 'kiwi'}; table.sort(t, function(a, b) if #a == #b then return a < b end return #a < #b end); return table.concat(t, ',')",
    );
    assert_eq!(returned_string(&state), "fig,kiwi,pear,banana");

    let (state, _gc) = compile_and_run(
        "local t = {10, 9, 8, 4}; table.sort(t, function(a, b) return a < b end, 'extra'); return table.concat(t, ',')",
    );
    assert_eq!(returned_string(&state), "4,8,9,10");
}

#[test]
fn table_sort_comparator_path_is_not_quadratic() {
    let (state, _gc) = compile_and_run(
        "local t = {}; for i = 1, 512 do t[i] = 513 - i end; local comparisons = 0; table.sort(t, function(a, b) comparisons = comparisons + 1; return a < b end); if t[1] == 1 and t[512] == 512 and comparisons < 20000 then return comparisons end return 0",
    );
    match return_value(&state) {
        Value::Number(n) => assert!(n > 0.0 && n < 20000.0),
        value => panic!("expected comparison count, got {value:?}"),
    }
}

#[test]
fn table_sort_default_comparator_uses_table_lt_metamethod() {
    let (state, _gc) = compile_and_run(
        "local mt = {__lt = function(a, b) return a.val < b.val end}; local t = {setmetatable({val = 5}, mt), setmetatable({val = 1}, mt), setmetatable({val = 3}, mt)}; table.sort(t); return t[1].val * 100 + t[2].val * 10 + t[3].val",
    );
    assert_eq!(return_value(&state), Value::Number(135.0));
}

#[test]
fn vm_comparisons_use_lt_and_le_metamethods() {
    let (state, _gc) = compile_and_run(
        "local mt = {__lt = function(a, b) return a.val < b.val end}; local a = setmetatable({val = 1}, mt); local b = setmetatable({val = 2}, mt); if a < b and a <= b and not (b < a) then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local mt = {__le = function(a, b) return a.rank <= b.rank end}; local a = setmetatable({rank = 3}, mt); local b = setmetatable({rank = 3}, mt); if a <= b then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn table_sort_results_pass_official_style_metamethod_check() {
    let (state, _gc) = compile_and_run(
        "local mt = {__lt = function(a, b) return a.val < b.val end}; local a = {}; for i = 1, 10 do a[i] = setmetatable({val = 11 - i}, mt) end; table.sort(a); for n = table.getn(a), 2, -1 do assert(not (a[n] < a[n - 1])) end; return a[1].val * 100 + a[10].val",
    );
    assert_eq!(return_value(&state), Value::Number(110.0));
}

#[test]
fn table_sort_reports_invalid_comparator() {
    let (state, _gc) = compile_and_run(
        "local ok, msg = pcall(table.sort, {1, 2}, 1); if not ok and string.match(msg, 'function expected') then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn string_basic_functions_work() {
    let (state, _gc) = compile_and_run("return string.len('hello')");
    assert_eq!(return_value(&state), Value::Number(5.0));

    let (state, _gc) = compile_and_run("return string.sub('abcdef', 2, -2)");
    assert_eq!(returned_string(&state), "bcde");

    let (state, _gc) = compile_and_run(
        "return table.concat({string.upper('ab'), string.lower('CD'), string.rep('x', 3), string.reverse('st')}, ':')",
    );
    assert_eq!(returned_string(&state), "AB:cd:xxx:ts");
}

#[test]
fn string_byte_and_char_use_lua_byte_semantics() {
    let (state, _gc) = compile_and_run(
        "local a, b, c = string.byte('\\0A\\255', 1, -1); if a == 0 and b == 65 and c == 255 and string.char(a, b, c) == '\\0A\\255' and #string.char(255) == 1 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local a = {string.byte('hi', 3, 4)}; if next(a) == nil and string.char() == '' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn member_access_works_after_rk_constant_limit() {
    let mut source = String::new();
    for i in 0..270 {
        source.push_str(&format!("rk_overflow_{i} = '{i}';"));
    }
    source.push_str("local t = {}; t.answer = 42; return string.byte('A') + t.answer");

    let (state, _gc) = compile_and_run(&source);
    assert_eq!(return_value(&state), Value::Number(107.0));
}

#[test]
fn string_format_supports_common_lua_printf_specs() {
    let (state, _gc) =
        compile_and_run("return string.format('Hello %s %02d/%d', 'World', '7', '2026')");
    assert_eq!(returned_string(&state), "Hello World 07/2026");

    let (state, _gc) = compile_and_run(
        "return string.format('[%8.2f]|%+05d|%.2e|%.4g|%#x|%#o|%c', 3.14159, 42, 1234, 1234.5678, 255, 9, 65)",
    );
    assert_eq!(
        returned_string(&state),
        "[    3.14]|+0042|1.23e+03|1235|0xff|011|A"
    );

    let (state, _gc) = compile_and_run(
        "local f = assert(loadstring(string.format('return %q', 'a\"b\\\\c'))); return f()",
    );
    assert_eq!(returned_string(&state), "a\"b\\c");

    let (state, _gc) = compile_and_run(
        "local f = assert(loadstring(string.format(\"return 'a%d'\", 42))); return f()",
    );
    assert_eq!(returned_string(&state), "a42");

    let (state, _gc) = compile_and_run(
        "local ok, msg = pcall(string.format, '%p', 1); if not ok and string.match(msg, 'invalid option') then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn string_find_returns_positions_and_supports_common_options() {
    let (state, _gc) =
        compile_and_run("local a, b = string.find('123456789', '345'); return a * 10 + b");
    assert_eq!(return_value(&state), Value::Number(35.0));

    let (state, _gc) =
        compile_and_run("local a, b = string.find('hello', '^hel'); return a * 10 + b");
    assert_eq!(return_value(&state), Value::Number(13.0));

    let (state, _gc) =
        compile_and_run("local a = string.find('1234567890123456789', '345', 4); return a");
    assert_eq!(return_value(&state), Value::Number(13.0));

    let (state, _gc) =
        compile_and_run("local a = string.find('1234567890123456789', '.45', -9); return a");
    assert_eq!(return_value(&state), Value::Number(13.0));

    let (state, _gc) =
        compile_and_run("local a, b = ('alo(.)alo'):find('(.)', 1, true); return a * 10 + b");
    assert_eq!(return_value(&state), Value::Number(46.0));

    let (state, _gc) =
        compile_and_run("if string.find('abcdef', 'xyz') == nil then return 1 end return 0");
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run("return string.find('', '')");
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run(
        "local s, e, cap = string.find('key=val', '(%a+)='); if cap == 'key' then return s * 10 + e end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(14.0));
}

#[test]
fn string_match_supports_lua_pattern_classes_captures_and_init() {
    let (state, _gc) = compile_and_run("return string.match('hello123', '%d+')");
    assert_eq!(returned_string(&state), "123");

    let (state, _gc) = compile_and_run(
        "local k, v = string.match('name=John', '(%a+)=(%a+)'); if k == 'name' and v == 'John' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) =
        compile_and_run("if string.match('hello', '%d+') == nil then return 1 end return 0");
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run("return string.match('abc 123 def', '%d+', 6)");
    assert_eq!(returned_string(&state), "23");

    let (state, _gc) = compile_and_run(
        "local a = ('abc123'):match('(%a+)%d+'); if a == 'abc' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state, _gc) = compile_and_run("return string.match('a (b (c) d) e', '%b()')");
    assert_eq!(returned_string(&state), "(b (c) d)");
}

#[test]
fn string_gsub_supports_pattern_replacement_counts_and_tables() {
    let (state, _gc) =
        compile_and_run("local r = string.gsub('abc 123 def 456', '%d+', 'NUM'); return r");
    assert_eq!(returned_string(&state), "abc NUM def NUM");

    let (state, _gc) =
        compile_and_run("local r, n = string.gsub('abc 123 def 456', '%d+', 'NUM'); return n");
    assert_eq!(return_value(&state), Value::Number(2.0));

    let (state, _gc) =
        compile_and_run("local r = string.gsub('hello world', '(%a+)', '[%1]'); return r");
    assert_eq!(returned_string(&state), "[hello] [world]");

    let (state, _gc) = compile_and_run("local r, n = ('aaa'):gsub('a', 'b', 2); return r");
    assert_eq!(returned_string(&state), "bba");

    let (state, _gc) = compile_and_run("local r, n = ('aaa'):gsub('a', 'b', 2); return n");
    assert_eq!(return_value(&state), Value::Number(2.0));

    let (state, _gc) = compile_and_run(
        "local r, n = string.gsub('a b c d', '(%a)', { a = 'A', b = false, c = nil, d = 'D' }); return r",
    );
    assert_eq!(returned_string(&state), "A b c D");

    let (state, _gc) = compile_and_run(
        "local r, n = string.gsub('a b c d', '(%a)', { a = 'A', b = false, c = nil, d = 'D' }); return n",
    );
    assert_eq!(return_value(&state), Value::Number(4.0));

    let (state, _gc) =
        compile_and_run("local r = string.gsub('foo bar', '%a+', { foo = 'FOO' }); return r");
    assert_eq!(returned_string(&state), "FOO bar");

    let (state, _gc) =
        compile_and_run("local r = string.gsub('ab', '(.)', { a = '%1', b = 7 }); return r");
    assert_eq!(returned_string(&state), "%17");

    let (state, _gc) = compile_and_run(
        "local t = {}; setmetatable(t, {__index = function (_, s) return string.upper(s) end}); local r = string.gsub('a alo b hi', '%w%w+', t); return r",
    );
    assert_eq!(returned_string(&state), "a ALO b HI");

    let (state, _gc) = compile_and_run(
        "local r, n = string.gsub('a=1 b=2 c=3', '(%a)=(%d)', function (k, v) if k == 'b' then return false end return k .. v .. v end); return r",
    );
    assert_eq!(returned_string(&state), "a11 b=2 c33");

    let (state, _gc) =
        compile_and_run("local r = string.gsub('aaa aa a', '%f[%w]a', 'x'); return r");
    assert_eq!(returned_string(&state), "xaa xa x");

    let (state, _gc) = compile_and_run(
        "if pcall(string.gsub, 'alo', '(.', print) then return 0 end; if pcall(string.gsub, 'alo', '.)', print) then return 0 end; if pcall(string.gsub, 'alo', '(.)', '%2') then return 0 end; if pcall(string.gsub, 'alo', '(%1)', 'a') then return 0 end; return 1",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn string_gmatch_iterates_matches_and_captures() {
    let (state, _gc) = compile_and_run(
        "local result = ''; local count = 0; for w in string.gmatch('hello world foo', '%a+') do result = result .. w .. ','; count = count + 1 end; if string.gfind == string.gmatch and count == 3 then return result end return 'bad'",
    );
    assert_eq!(returned_string(&state), "hello,world,foo,");

    let (state, _gc) = compile_and_run(
        "local result = ''; local count = 0; for n in string.gmatch('abc 123 def 456 ghi 789', '%d+') do result = result .. n .. ','; count = count + 1 end; if count == 3 then return result end return 'bad'",
    );
    assert_eq!(returned_string(&state), "123,456,789,");

    let (state, _gc) = compile_and_run(
        "local keys = ''; local vals = ''; local count = 0; for k, v in string.gmatch('name=John age=30 city=NYC', '(%w+)=(%w+)') do keys = keys .. k .. ','; vals = vals .. v .. ','; count = count + 1 end; if count == 3 and vals == 'John,30,NYC,' then return keys end return 'bad'",
    );
    assert_eq!(returned_string(&state), "name,age,city,");

    let (state, _gc) = compile_and_run(
        "local result = ''; local count = 0; for c in ('abc'):gmatch('.') do result = result .. c; count = count + 1 end; if count == 3 then return result end return 'bad'",
    );
    assert_eq!(returned_string(&state), "abc");

    let (state, _gc) = compile_and_run(
        "local count = 0; for w in string.gmatch('hello world', '%d+') do count = count + 1 end; return count",
    );
    assert_eq!(return_value(&state), Value::Number(0.0));
}

#[test]
fn next_pairs_and_ipairs_work() {
    let (state, _gc) = compile_and_run("local t = {7, 8}; local k, v = next(t); return k * 10 + v");
    assert_eq!(return_value(&state), Value::Number(17.0));

    let (state, _gc) = compile_and_run(
        "local sum = 0; for i, v in ipairs({3, 4, 5}) do sum = sum + i * v end; return sum",
    );
    assert_eq!(return_value(&state), Value::Number(26.0));

    let (state, _gc) = compile_and_run(
        "local sum = 0; for k, v in pairs({a = 10, b = 20}) do sum = sum + v end; return sum",
    );
    assert_eq!(return_value(&state), Value::Number(30.0));

    let (state, _gc) = compile_and_run(
        "local function find1(name) for n, v in pairs(_G) do if n == name then return v end end end; if find1('assert') == assert and find1('print') == print then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn table_getn_ignores_n_field_for_insert_remove_compatibility() {
    let (state, _gc) = compile_and_run(
        "local a = {n = 0, [-7] = 'ban'}; table.insert(a, 10); table.insert(a, 2, 20); table.insert(a, 1, -1); table.insert(a, 40); table.insert(a, table.getn(a) + 1, 50); table.insert(a, 2, -2); local ok = table.remove(a, 1) == -1 and table.remove(a, 1) == -2 and table.remove(a, 1) == 10 and table.remove(a, 1) == 20 and table.remove(a, 1) == 40 and table.remove(a, 1) == 50 and table.remove(a, 1) == nil; if ok and a.n == 0 and a[-7] == 'ban' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn setfenv_numeric_level_returns_target_function() {
    let (state, _gc) = compile_and_run(
        "local _G = _G; local g; local function f() return setfenv(2, {a = '10'}) end; g = function() local r = f(); if r ~= g then return 0 end; return _G.getfenv(1).a == '10' and 1 or 0 end; if g() == 1 and getfenv(g).a == '10' then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn top_level_setfenv_uses_chunk_environment() {
    let (state, _gc) = compile_and_run(
        "X = 20; setfenv(1, setmetatable({}, {__index = _G})); X = X + 10; if X == 30 and _G.X == 20 and getfenv(0) == _G then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn metatable_call_arithmetic_concat_and_eq_work() {
    let (state, _gc) = compile_and_run(
        "local t = {}; t.__call = function(self, ...) return {...} end; t.__add = function(a, b) return a end; t.__concat = function(a, b) return 'joined' end; t.__eq = function(a, b) return true end; local a = setmetatable({val = 'a'}, t); local b = setmetatable({val = 'b'}, t); local r = a(1, 2, 3); if r[1] == 1 and r[3] == 3 and a + 5 == a and a .. b == 'joined' and a == b then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn newproxy_and_basic_type_metatables_work() {
    let (state, _gc) = compile_and_run(
        "local u = newproxy(true); getmetatable(u).__newindex = function(obj, k, v) getmetatable(obj)[k] = v end; getmetatable(u).__index = function(obj, k) return getmetatable(obj)[k] end; u[7] = 70; local k = newproxy(u); local mt = {}; debug.setmetatable(10, mt); mt.__index = function(a, b) return a + b end; mt.__add = function(a, b) return (a or 0) + (b or 0) end; if u[7] == 70 and getmetatable(k) == getmetatable(u) and (10)[3] == 13 then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn generic_for_accepts_lua_iterator_functions() {
    let (state, _gc) = compile_and_run(
        "local function countdown(n) return function(state, current) current = current + 1; if current <= state then return current, state - current end end, n, 0 end; local sum = 0; for i, v in countdown(3) do sum = sum + i * 10 + v end; return sum",
    );
    assert_eq!(return_value(&state), Value::Number(63.0));

    let (state, _gc) = compile_and_run(
        "local function upto(n) local i = 0; return function() i = i + 1; if i <= n then return i end end end; local sum = 0; for i in upto(4) do sum = sum + i end; return sum",
    );
    assert_eq!(return_value(&state), Value::Number(10.0));
}

#[test]
fn alien_signals_example_script_runs() {
    let dir = alien_signals_dir();
    let example_path = dir.join("example.lua");
    assert!(
        example_path.is_file(),
        "alien_signals example should exist at {}",
        example_path.display()
    );
    let example_lit = lua_path_literal(&example_path);
    let source = format!(
        "arg = {{ [0] = {example_lit} }}; local chunk = assert(loadfile({example_lit})); chunk(); return 1"
    );

    let (state, _gc) = compile_and_run(&source);
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn alien_signals_init_entry_loads_and_updates_reactive_graph() {
    let dir = alien_signals_dir();
    assert!(dir.join("init.lua").is_file());
    let dir_lit = lua_path_literal(&dir);
    let source = format!(
        r#"
        local dir = {dir_lit}
        package.path = dir .. "/?.lua;" .. dir .. "/?/init.lua;" .. package.path

        local function flatLoader(name)
            return function()
                return assert(loadfile(dir .. "/" .. name .. ".lua"))()
            end
        end

        -- init.lua is the public module entry; the copied tests keep the
        -- refactored.* modules flat, so the harness mirrors example.lua's loader.
        package.preload["refactored"] = function()
            return assert(loadfile(dir .. "/init.lua"))()
        end
        package.preload["refactored.constants"] = flatLoader("constants")
        package.preload["refactored.tracer"] = flatLoader("tracer")
        package.preload["refactored.scheduler"] = flatLoader("scheduler")
        package.preload["refactored.graph"] = flatLoader("graph")
        package.preload["refactored.engine"] = flatLoader("engine")
        package.preload["refactored.primitives"] = flatLoader("primitives")

        local s = require("refactored")
        local count = s.signal(1, "count")
        local doubled = s.computed(function()
            return count() * 2
        end, "doubled")
        local observed = {{}}
        local stop = s.effect(function()
            observed[#observed + 1] = doubled()
        end, "collector")

        count(3)
        local ok =
            s.isSignal(count)
            and s.isComputed(doubled)
            and s.isEffect(stop)
            and observed[1] == 2
            and observed[2] == 6
            and doubled() == 6

        stop()
        count(4)
        if ok and observed[3] == nil then
            return 1
        end
        return 0
        "#
    );

    let (state, _gc) = compile_and_run(&source);
    assert_eq!(return_value(&state), Value::Number(1.0));
}
