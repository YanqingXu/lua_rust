//! Fixed faults and actual compiler/binary; this executable never imports the learner hook.
#[path = "../exercises/repl_classification/faults.rs"]
mod faults;
#[path = "../exercises/repl_classification/model.rs"]
mod model;
#[path = "../exercises/support/process.rs"]
mod process;
#[path = "../exercises/repl_classification/real.rs"]
mod real;
#[path = "../exercises/repl_classification/solution.rs"]
mod solution;

use lua_compiler::compile;
use model::Decision;

fn keywords() {
    let compilation = compile(b"return 40 +\n");
    assert_eq!(faults::keywords(&compilation), Decision::Reject);
    assert_eq!(solution::decide(&compilation), Decision::Preserve);
    println!(
        "before keywords: expression=Reject appended-chunk-compiles={}",
        compile(b"return 40 +\n2\n").is_ok()
    );
    println!(
        "after structured: expression=Preserve result={:?}",
        model::complete(b"return 40 +\n2\n").unwrap()
    );
    let decide: model::Decide = solution::decide;
    for (name, prefix, suffix) in model::unfinished() {
        assert_eq!(decide(&compile(prefix)), Decision::Preserve);
        assert!(compile(&[prefix, suffix].concat()).is_ok());
        println!("after {name}: NeedMoreInput=Preserve appended-chunk-compiles=true");
    }
}

fn eof() {
    for (name, source) in model::syntax_errors() {
        let compilation = compile(source);
        assert_eq!(solution::decide(&compilation), Decision::Reject);
        let error = compilation.as_ref().unwrap_err();
        println!(
            "after {name}: kind={:?} decision=Reject at-eof={}",
            error.kind,
            error.position.offset == source.len()
        );
    }
    let source = b"return 1e";
    let compilation = compile(source);
    assert_eq!(
        faults::eof_position(&compilation, source.len()),
        Decision::Preserve
    );
    println!("before EOF-position: bad-exponent=Preserve after=Reject appended-newline-does-not-repair={}", compile(b"return 1e\n2\n").is_err());
}

fn wording() {
    let mut pending = compile(b"return 40 +\n").unwrap_err();
    pending.message = "please finish this expression".into();
    assert_eq!(solution::decide(&Err(pending)), Decision::Preserve);
    let mut invalid = compile(b"return @\n").unwrap_err();
    invalid.message = "unterminated text still describes an invalid token".into();
    let compilation = Err(invalid);
    assert_eq!(faults::keywords(&compilation), Decision::Preserve);
    assert_eq!(solution::decide(&compilation), Decision::Reject);
    println!("wording-only mutation: NeedMoreInput=Preserve Invalid=Reject keywords-false-positive=Preserve");
}

fn real() -> Result<(), String> {
    for case in real::cases(&process::example_binary()?)? {
        println!(
            "real {}: exit=0 exact-stdout=true stdout={:?} diagnostics={}",
            case.name,
            String::from_utf8_lossy(&case.stdout),
            String::from_utf8_lossy(&case.stderr)
                .matches("lua: ")
                .count()
        );
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "keywords" => keywords(),
        "eof" => eof(),
        "wording" => wording(),
        "real" => real()?,
        "all" => {
            keywords();
            eof();
            wording();
            real()?;
        }
        _ => {
            return Err(
                "usage: repl_classification_problem_chain [keywords|eof|wording|real|all]".into(),
            )
        }
    }
    Ok(())
}
