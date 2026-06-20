//! lua_app — Lua 5.1 CLI application
//!
//! Runs Lua scripts from files or interactive REPL.
//! C++ reference: `lua_cpp/src/main.cpp`, `lua_cpp/src/repl/`.

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_stdlib::catalog::open_all;
use lua_vm::execute::execute_proto;
use lua_vm::state::LuaState;

use std::env;
use std::fs;
use std::io::{self, Write};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        repl();
    } else {
        let filename = &args[1];
        match run_file(filename) {
            Ok(_) => {}
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}

/// Run a Lua source file
fn run_file(filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(filename)?;
    run_source(&source, filename)
}

/// Run Lua source code
pub fn run_source(source: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Lua 5.1 Rust Interpreter ===");
    println!("Source: {} ({} bytes)", name, source.len());

    // Phase 2: Parse
    let mut parser = Parser::new(source);
    let chunk = match parser.parse() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            return Err(e.into());
        }
    };
    println!("Parsed: {} top-level statements", chunk.statements.len());

    // Phase 2: CodeGen
    let cg = CodeGenerator::new();
    let proto = match cg.generate(&chunk, name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("CodeGen error: {}", e);
            return Err(e.into());
        }
    };
    println!(
        "Compiled: {} instructions, {} constants",
        proto.instruction_count(),
        proto.constant_count()
    );

    // Phase 3: VM Execution
    let mut state = LuaState::new();

    // Phase 4: Load standard library
    open_all(&mut state);

    println!("--- Execution ---");
    match execute_proto(&mut state, &proto) {
        Ok(result) => println!("VM result: {:?} (status: {:?})", result, state.get_status()),
        Err(e) => eprintln!("Runtime error: {}", e),
    }

    Ok(())
}

/// Interactive REPL
fn repl() {
    println!("Lua 5.1 Rust Interpreter REPL");
    println!("Type 'exit' to quit, 'help' for commands.");
    println!();

    let mut state = LuaState::new();
    open_all(&mut state);

    let mut buffer = String::new();
    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        buffer.clear();
        if io::stdin().read_line(&mut buffer).is_err() {
            break;
        }

        let line = buffer.trim();
        match line {
            "exit" | "quit" => break,
            "help" => {
                println!("Commands:");
                println!("  <lua code>  — Execute Lua expression/statement");
                println!("  exit/quit   — Exit REPL");
                println!("  help        — Show this help");
                continue;
            }
            "" => continue,
            _ => {}
        }

        // Wrap expressions in 'return' for display
        let source = if !line.starts_with("return ")
            && !line.starts_with("local ")
            && !line.starts_with("if ")
            && !line.starts_with("for ")
            && !line.starts_with("while ")
            && !line.starts_with("repeat ")
            && !line.starts_with("do ")
            && !line.starts_with("function ")
        {
            format!("return {}", line)
        } else {
            line.to_string()
        };

        match run_repl_line(&mut state, &source) {
            Ok(_) => {}
            Err(e) => eprintln!("Error: {}", e),
        }
    }
    println!("Goodbye.");
}

fn run_repl_line(state: &mut LuaState, source: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut parser = Parser::new(source);
    let chunk = parser.parse()?;
    let cg = CodeGenerator::new();
    let proto = cg.generate(&chunk, "<repl>")?;

    match execute_proto(state, &proto) {
        Ok(_result) => {
            // Print stack top as result
            if let Some(val) = state.pop() {
                println!("{}", format_value(&val));
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn format_value(v: &lua_core::value::Value) -> String {
    use lua_core::value::Value;
    match v {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(_) => "<string>".to_string(),
        Value::Table(_) => "<table>".to_string(),
        Value::Function(_) => "<function>".to_string(),
        Value::Userdata(_) => "<userdata>".to_string(),
        Value::Thread(_) => "<thread>".to_string(),
        Value::LightUserdata(_) => "<lightuserdata>".to_string(),
    }
}
