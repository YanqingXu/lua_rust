//! 固定重放目录/安装反例；不读取 starter，退出 0 不表示练习完成。
#[path = "../exercises/catalog/faults.rs"]
mod faults;
#[path = "../exercises/catalog/model.rs"]
pub mod model;
#[path = "../exercises/catalog/real.rs"]
mod real;
#[path = "../exercises/catalog/solution.rs"]
mod solution;

use lua_core::{MathFunction, NativeFunction};
use lua_stdlib::{FunctionSpec, LibrarySpec, BASE_FUNCTIONS, LIBRARIES};

fn missing() {
    let before = faults::omit_sqrt(BASE_FUNCTIONS, LIBRARIES).unwrap();
    let after = solution::install(BASE_FUNCTIONS, LIBRARIES).unwrap();
    println!(
        "before missing: math.sqrt={}",
        before.lookup(b"math", b"sqrt").unwrap()
    );
    println!(
        "after installed: math.sqrt(81)={:?}",
        real::installed_call(&after, b"math", b"sqrt", b"local f=0; return f(81)")
    );
    println!("after binding_count={}", after.snapshot().unwrap().len());
}

fn identity() {
    let before = faults::wrong_sqrt_id(BASE_FUNCTIONS, LIBRARIES).unwrap();
    let after = solution::install(BASE_FUNCTIONS, LIBRARIES).unwrap();
    println!(
        "before wrong-id: math.sqrt(81)={:?}",
        real::installed_call(&before, b"math", b"sqrt", b"local f=0; return f(81)")
    );
    println!(
        "after right-id: math.sqrt(81)={:?}",
        real::installed_call(&after, b"math", b"sqrt", b"local f=0; return f(81)")
    );
}

fn duplicates() {
    const FIRST: FunctionSpec = FunctionSpec {
        name: b"same",
        function: NativeFunction::Math(MathFunction::Abs),
    };
    const SECOND: FunctionSpec = FunctionSpec {
        name: b"same",
        function: NativeFunction::Math(MathFunction::Sqrt),
    };
    const BAD: &[FunctionSpec] = &[FIRST, SECOND];
    let error = model::preflight(
        &[],
        &[LibrarySpec {
            name: b"math",
            functions: BAD,
        }],
    )
    .unwrap_err();
    println!("teaching preflight same namespace: {error}");
    let libraries = [
        LibrarySpec {
            name: b"left",
            functions: &[FIRST],
        },
        LibrarySpec {
            name: b"right",
            functions: &[SECOND],
        },
    ];
    let installed = solution::install(&[], &libraries).unwrap();
    println!(
        "cross namespace: left.same(81)={:?} right.same(81)={:?}",
        real::installed_call(&installed, b"left", b"same", b"local f=0; return f(81)"),
        real::installed_call(&installed, b"right", b"same", b"local f=0; return f(81)")
    );
}

fn real() {
    println!(
        "installed IDs / real VM bodies={:?}",
        real::installed_effects(&solution::install(BASE_FUNCTIONS, LIBRARIES).unwrap())
    );
    println!("real VM public effects={:?}", real::public_vm_effects());
    println!(
        "real VM separate diagnostics={}",
        real::diagnostic_is_separately_installed()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "missing" => missing(),
        "identity" => identity(),
        "duplicates" => duplicates(),
        "real" => real(),
        "all" => {
            missing();
            identity();
            duplicates();
            real();
        }
        _ => {
            eprintln!("usage: catalog_problem_chain [missing|identity|duplicates|real|all]");
            std::process::exit(2);
        }
    }
}
