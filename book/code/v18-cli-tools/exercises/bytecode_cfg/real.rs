use crate::contracts::equal;
use crate::model::{edge, graph, Graph, Leaders};
use crate::{bytecode_process, process};
use lua_compiler::compile;
use lua_core::Proto;
use std::path::Path;

// ANCHOR: cfg_real_sources
pub const IF_SOURCE: &[u8] = b"if flag then return 1 end\nreturn 2";
pub const LOOP_SOURCE: &[u8] = b"local total=0\nfor i=1,2 do total=total+i end\nreturn total";
// ANCHOR_END: cfg_real_sources

pub struct Compiled {
    pub name: &'static str,
    pub source: &'static [u8],
    pub proto: Proto,
    pub expected: Graph,
}

pub fn compiled_cases() -> Result<Vec<Compiled>, String> {
    [
        (
            "if",
            IF_SOURCE,
            Graph {
                blocks: vec![(0, 2), (2, 5), (5, 7), (7, 8), (8, 11), (11, 12)],
                edges: vec![
                    edge(0, 3, "jump"),
                    edge(0, 1, "next"),
                    edge(2, 4, "jump"),
                    edge(3, 4, "next"),
                ],
            },
        ),
        (
            "for",
            LOOP_SOURCE,
            Graph {
                blocks: vec![(0, 9), (9, 14), (14, 15), (15, 19), (19, 20)],
                edges: vec![
                    edge(0, 2, "jump"),
                    edge(1, 2, "next"),
                    edge(2, 1, "jump"),
                    edge(2, 3, "next"),
                ],
            },
        ),
    ]
    .into_iter()
    .map(|(name, source, expected)| {
        let proto = compile(source).map_err(|error| error.to_string())?;
        equal(
            "compiled instruction count",
            proto.code.len(),
            expected.blocks.last().unwrap().1,
        )?;
        Ok(Compiled {
            name,
            source,
            proto,
            expected,
        })
    })
    .collect()
}

pub fn compiler_graphs(discover: Leaders) -> Result<Vec<(&'static str, Graph)>, String> {
    let mut observed = Vec::new();
    for case in compiled_cases()? {
        let actual = graph(&case.proto, discover)?;
        equal(case.name, actual.clone(), case.expected.clone())?;
        equal(
            "complete public DOT",
            lua_bytecode::cfg_dot(&case.proto)?,
            case.expected.dot(&case.proto)?,
        )?;
        observed.push((case.name, actual));
    }
    Ok(observed)
}

pub fn binary_graphs(lua_app: &Path) -> Result<Vec<(&'static str, String)>, String> {
    let binary = bytecode_process::sibling_of_lua_app(lua_app)?;
    binary_at(&binary)
}

pub fn binary_at(binary: &Path) -> Result<Vec<(&'static str, String)>, String> {
    let mut observed = Vec::new();
    for case in compiled_cases()? {
        let output = process::run(
            binary,
            &["--format".to_owned(), "cfg".to_owned(), "-".to_owned()],
            case.source,
        )?;
        equal("bytecode child exit", output.status.code(), Some(0))?;
        equal("bytecode child stderr", output.stderr, vec![])?;
        // Compare bytes to the complete public rendering, not a keyword substring.
        let public = lua_bytecode::cfg_dot(&case.proto)?;
        equal(
            "public graph matches exact expected blocks/edges",
            public.clone(),
            case.expected.dot(&case.proto)?,
        )?;
        equal(
            "bytecode child complete stdout",
            output.stdout,
            public.clone().into_bytes(),
        )?;
        observed.push((case.name, public));
    }
    Ok(observed)
}
