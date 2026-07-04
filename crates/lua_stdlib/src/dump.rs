//! Internal dump/undump registry for Lua functions.
//!
//! This is not Lua's binary chunk format. It gives `string.dump` and
//! `load/loadstring` a reversible in-process representation while the real
//! binary serializer is still pending.

use std::cell::RefCell;
use std::collections::HashMap;

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;
use lua_vm::state::LuaState;

const DUMP_PREFIX: &str = "\u{1b}LuaRustDump:";

thread_local! {
    static NEXT_DUMP_ID: RefCell<usize> = const { RefCell::new(1) };
    static DUMPS: RefCell<HashMap<usize, GcRef<Proto>>> = RefCell::new(HashMap::new());
    static SOURCES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

pub fn remember_function_source(func_ref: GcRef<Function>, source: &str) {
    // SAFETY: the function was just created by the active VM/compiler.
    let Some(func) = (unsafe { func_ref.as_ref() }) else {
        return;
    };
    let Some(proto) = func.proto() else {
        return;
    };
    SOURCES.with(|sources| {
        sources
            .borrow_mut()
            .insert(proto.as_ptr() as usize, source.to_string());
    });
}

pub fn dump_function(func_ref: GcRef<Function>) -> Option<String> {
    // SAFETY: the function being dumped is an argument on the active Lua stack.
    let func = unsafe { func_ref.as_ref() }?;
    if !func.is_lua_function() {
        return None;
    }
    let proto = func.proto()?;
    let id = NEXT_DUMP_ID.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next += 1;
        id
    });
    DUMPS.with(|dumps| {
        dumps.borrow_mut().insert(id, proto);
    });

    let source = SOURCES.with(|sources| sources.borrow().get(&(proto.as_ptr() as usize)).cloned());
    Some(match source {
        Some(source) => format!("{DUMP_PREFIX}{id}:{}", hex_encode(source.as_bytes())),
        None => format!("{DUMP_PREFIX}{id}"),
    })
}

pub fn undump_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    source: &str,
) -> Option<Result<GcRef<Function>, String>> {
    let payload = source.strip_prefix(DUMP_PREFIX)?;
    let (id_text, encoded_source) = payload
        .split_once(':')
        .map_or((payload, None), |(id, encoded)| (id, Some(encoded)));
    let id = match id_text.parse::<usize>() {
        Ok(id) => id,
        Err(_) => return Some(Err("invalid dumped function".to_string())),
    };
    let proto = DUMPS.with(|dumps| dumps.borrow().get(&id).copied());
    let Some(proto_ref) = proto else {
        return Some(match encoded_source {
            Some(encoded) => compile_dumped_source(l, gc, encoded),
            None => Err("unknown dumped function".to_string()),
        });
    };

    // SAFETY: dump registry only stores live protos produced by this process.
    let Some(proto) = (unsafe { proto_ref.as_ref() }) else {
        return Some(Err("invalid dumped proto".to_string()));
    };
    let mut function = Function::new_lua(proto_ref);
    for _ in 0..proto.num_upvalues() {
        let upvalue = gc.create(Upvalue::new_closed(Value::Nil));
        function.add_upvalue(upvalue);
    }
    Some(Ok(gc.create(function)))
}

fn compile_dumped_source(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    encoded_source: &str,
) -> Result<GcRef<Function>, String> {
    let source = hex_decode(encoded_source)?;
    let mut parser = Parser::new(&source);
    let chunk = parser
        .parse()
        .map_err(|err| format!("=(dump):{}: {}", err.line, err.message))?;

    let mut generator = CodeGenerator::new(gc);
    if let Some(pool_ptr) = l.string_pool {
        // SAFETY: LuaState::string_pool is owned by the host for this VM call.
        generator.builder.bind_pool(unsafe { &mut *pool_ptr });
    }
    let proto = generator
        .generate(&chunk, "=(dump)")
        .map_err(|err| format!("=(dump):{err}"))?;
    let proto_ref = gc.create(proto);
    let mut function = Function::new_lua(proto_ref);
    function.set_env(l.thread_env.or(l.global_table));
    Ok(gc.create(function))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Result<String, String> {
    if text.len() % 2 != 0 {
        return Err("invalid dumped source".to_string());
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let chars = text.as_bytes();
    for pair in chars.chunks_exact(2) {
        let high = hex_value(pair[0]).ok_or_else(|| "invalid dumped source".to_string())?;
        let low = hex_value(pair[1]).ok_or_else(|| "invalid dumped source".to_string())?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| "invalid dumped source".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
