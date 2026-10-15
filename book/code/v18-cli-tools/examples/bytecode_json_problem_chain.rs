//! Fixed faults/reference and actual Proto views; learner starter is never imported.
#[path = "../exercises/support/bytecode_process.rs"]
mod bytecode_process;
#[path = "../exercises/bytecode_json/faults.rs"]
mod faults;
#[path = "../exercises/bytecode_json/model.rs"]
mod model;
#[path = "../exercises/support/process.rs"]
mod process;
#[path = "../exercises/bytecode_json/real.rs"]
mod real;
#[path = "../exercises/bytecode_json/solution.rs"]
mod solution;
#[path = "../exercises/support/temp.rs"]
mod temp;

fn bytes() {
    let raw = faults::raw_text(b"\"\\\0\n");
    println!(
        "before raw-injection: json={raw:?} contains-control={}",
        raw.bytes().any(|byte| byte < 32)
    );
    let left = faults::lossy_text(&[255]);
    let right = faults::lossy_text(&[254]);
    println!(
        "before lossy: ff={left} fe={right} collision={}",
        left == right
    );
    let encode: model::Encode = solution::encode_bytes;
    for (label, bytes) in model::cases() {
        let json = encode(&bytes);
        assert_eq!(model::decode_constant(&json), Ok(bytes.clone()));
        println!(
            "after {label}: json={json} exact=true bytes={}",
            bytes.len()
        );
    }
}

fn proto() {
    assert_eq!(real::document(), real::expected_document());
    println!("real Proto: constants=13 nested=1 code-fields=2 nonfinite=[NaN,inf,-inf] metadata-byte-escape=true");
}

fn real() -> Result<(), String> {
    for probe in real::binary_documents_at(&bytecode_process::example_binary()?)? {
        println!(
            "real process {}: exit=0 stderr=empty exact-public-render=true bytes={}",
            probe.mode,
            probe.stdout.len()
        );
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "bytes" => bytes(),
        "proto" => proto(),
        "real" => real()?,
        "document" => print!("{}", real::document()),
        "all" => {
            bytes();
            proto();
            real()?;
        }
        _ => {
            return Err(
                "usage: bytecode_json_problem_chain [bytes|proto|real|document|all]".to_owned(),
            )
        }
    }
    Ok(())
}
