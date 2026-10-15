//! fixed replay，不调用学习者 starter：运行成功不表示已完成 TODO。
#[path = "../exercises/callbacks/faults.rs"]
mod faults;
#[path = "../exercises/callbacks/model.rs"]
mod model;
#[path = "../exercises/callbacks/real.rs"]
mod real;
#[path = "../exercises/callbacks/solution.rs"]
mod solution;

use model::{CallbackVm, Design};

fn healthy() -> Design {
    Design {
        take: solution::take_batch,
        pin: solution::pin_batch,
        drain: solution::drain_batch,
    }
}

fn replay(label: &str, mut vm: CallbackVm) {
    let result = vm.drain_pending();
    println!("{label}: {}", vm.summary(&result));
    for line in &vm.trace {
        println!("  {line}");
    }
    println!("  sibling_live={}/{}", vm.is_live(2), vm.is_live(102));
    assert_eq!(vm.roots().first(), Some(&900));
    vm.collect();
    println!("  after-next-gc sibling_live={}", vm.is_live(2));
}

fn ownership() {
    replay(
        "before clone",
        CallbackVm::reentrant_pair(Design {
            take: faults::clone_batch,
            ..healthy()
        }),
    );
    replay("after take", CallbackVm::reentrant_pair(healthy()));
}

fn roots() {
    replay(
        "before current-only",
        CallbackVm::reentrant_pair(Design {
            pin: faults::current_only,
            ..healthy()
        }),
    );
    replay("after batch-roots", CallbackVm::reentrant_pair(healthy()));
}

fn errors() {
    replay(
        "before question-mark",
        CallbackVm::failing_trio(Design {
            drain: faults::early_return,
            ..healthy()
        }),
    );
    replay("after first-error", CallbackVm::failing_trio(healthy()));
}

fn real() {
    println!(
        "real Heap current-only: userdata/closure={:?}",
        real::heap_owned_batch_accessible(false)
    );
    println!(
        "real Heap batch-roots: userdata/closure={:?}",
        real::heap_owned_batch_accessible(true)
    );
    println!("real VM reentry: {}", real::vm_reentrant_pair());
    println!("real VM error/sibling: {:?}", real::vm_error_then_sibling());
}

fn main() {
    let selected = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    match selected.as_str() {
        "ownership" => ownership(),
        "roots" => roots(),
        "errors" => errors(),
        "real" => real(),
        "all" => {
            ownership();
            roots();
            errors();
            real();
        }
        _ => {
            eprintln!("usage: finalizer_problem_chain [ownership|roots|errors|real|all]");
            std::process::exit(2);
        }
    }
}
