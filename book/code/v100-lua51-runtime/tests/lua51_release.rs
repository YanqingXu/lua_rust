use lua_app::{execute, execute_multi};
use lua_core::{ByteString, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn tsv_rows(input: &str) -> Vec<Vec<&str>> {
    input
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect())
        .collect()
}

#[test]
fn lua51_call_sugar_and_parenthesized_prefix_statements_execute() {
    assert_eq!(
        execute(
            br#"
                local capture = function(value) return value end
                local from_string = capture "lua"
                local from_table = capture { answer = 42 }
                local side_effect = 0;
                (function() side_effect = 1 end)()
                return from_string, from_table.answer, side_effect
            "#,
        )
        .unwrap(),
        Value::String(ByteString::new(b"lua".as_slice()))
    );
    assert_eq!(
        execute_multi(
            br#"
                local capture = function(value) return value end
                local from_string = capture "lua"
                local from_table = capture { answer = 42 }
                local side_effect = 0;
                (function() side_effect = 1 end)()
                return from_string, from_table.answer, side_effect
            "#,
        )
        .unwrap(),
        vec![
            Value::String(ByteString::new(b"lua".as_slice())),
            Value::Number(42.0),
            Value::Number(1.0),
        ]
    );
}

#[test]
fn capability_matrix_and_deviation_registry_are_closed_world() {
    let capabilities = tsv_rows(include_str!("../compatibility/capabilities.tsv"));
    let deviations = tsv_rows(include_str!("../compatibility/deviations.tsv"));
    assert_eq!(
        capabilities[0],
        ["category", "feature", "status", "evidence", "deviation"]
    );
    assert_eq!(
        deviations[0],
        ["id", "scope", "status", "target", "description"]
    );

    let registry = deviations
        .iter()
        .skip(1)
        .map(|row| {
            assert_eq!(row.len(), 5);
            assert!(row[0].starts_with("LUA51-D"));
            assert!(["approved", "registered", "deferred"].contains(&row[2]));
            (row[0], row[2])
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        registry.len(),
        deviations.len() - 1,
        "duplicate deviation id"
    );

    let mut referenced = BTreeSet::new();
    for row in capabilities.iter().skip(1) {
        assert_eq!(row.len(), 5);
        assert!(["complete", "partial", "excluded"].contains(&row[2]));
        assert!(!row[3].is_empty());
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(row[3]).exists(),
            "missing evidence path {}",
            row[3]
        );
        if row[2] == "complete" {
            assert_eq!(row[4], "-", "complete rows cannot hide a deviation");
        } else {
            assert!(
                registry.contains_key(row[4]),
                "unknown deviation {}",
                row[4]
            );
            referenced.insert(row[4]);
        }
    }
    assert_eq!(
        referenced.len(),
        registry.len(),
        "every registered deviation must be visible in the capability matrix"
    );
    assert!(capabilities.len() >= 25);
}

#[test]
fn oracle_identity_and_post_v1_boundaries_are_pinned() {
    let manifest = include_str!("../compatibility/oracles.toml");
    assert!(manifest.contains("Lua 5.1.5"));
    assert!(manifest.contains("2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333"));
    assert!(manifest.contains("87c15e69ceb94eb74e28226ccbefb7e196635711"));
    assert!(manifest.contains("66c9506a07c4bab8b8e84d7858484e1456772bf605be2ae40003a22a8d1bbfb8"));
    assert!(manifest.contains("unlisted_difference = \"fail\""));

    let baseline = include_str!("../compatibility/differential-baseline.json");
    assert!(baseline.contains("\"passed\": true"));
    assert_eq!(baseline.matches("\"exit_code\": 0").count(), 5);
    assert!(baseline.contains("lua-rust-lua51-differential-baseline/v1"));

    let deviations = include_str!("../compatibility/deviations.tsv");
    assert!(deviations.contains("LUA51-D001\tbinary-chunk\tdeferred\tv1.1"));
    assert!(deviations.contains("LUA51-D002\tc-api\tdeferred\tv1.2"));
    assert!(deviations.contains("LUA51-D003\tnative-module\tdeferred\tv1.3"));
}
