//! Full-REPL submitted-line completion.

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::runtime::Runtime;
use lua_vm::state::LuaState;

pub(crate) struct SubmittedCompletion {
    pub(crate) line: String,
    pub(crate) stdout: String,
}

pub(crate) struct CompletionResult {
    pub(crate) line: String,
    pub(crate) candidates: Vec<String>,
}

pub(crate) fn apply_submitted(
    runtime: &mut Runtime,
    submitted: &str,
) -> Result<SubmittedCompletion, String> {
    let mut line = submitted.to_string();
    let mut stdout = String::new();
    while let Some(tab) = line.find('\t') {
        let before = &line[..tab];
        let after = &line[tab + 1..];
        let completion = complete(runtime, before)?;
        line = completion.line + after;
        if completion.candidates.len() > 1 {
            stdout.push('\n');
            stdout.push_str(&completion.candidates.join("  "));
            stdout.push('\n');
        }
    }
    Ok(SubmittedCompletion { line, stdout })
}

pub(crate) fn complete(runtime: &mut Runtime, line: &str) -> Result<CompletionResult, String> {
    let first = line
        .char_indices()
        .find(|(_, character)| !character.is_ascii_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(line.len());
    if line.as_bytes().get(first) == Some(&b'.') {
        return Ok(complete_meta(line, first));
    }

    let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
    let (state, gc, _) = parts.split_mut();
    complete_lua(state, gc, line)
}

fn complete_meta(line: &str, command_start: usize) -> CompletionResult {
    let command_end = line[command_start..]
        .find(char::is_whitespace)
        .map_or(line.len(), |offset| command_start + offset);
    if command_end == line.len() {
        let token = &line[command_start..];
        return build_result(
            line,
            command_start,
            token,
            [".ast", ".bytecode", ".gc", ".help"]
                .into_iter()
                .filter(|candidate| candidate.starts_with(token))
                .map(ToOwned::to_owned)
                .collect(),
        );
    }

    if &line[command_start..command_end] != ".gc" {
        return no_completion(line);
    }
    let option_start = line[..]
        .char_indices()
        .rev()
        .find(|(index, character)| *index >= command_end && character.is_ascii_whitespace())
        .map_or(command_end, |(index, character)| {
            index + character.len_utf8()
        });
    let token = &line[option_start..];
    build_result(
        line,
        option_start,
        token,
        ["collect", "help", "stats", "status", "strategy"]
            .into_iter()
            .filter(|candidate| candidate.starts_with(token))
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn complete_lua(
    state: &LuaState,
    gc: &GarbageCollector,
    line: &str,
) -> Result<CompletionResult, String> {
    let token_start = line
        .char_indices()
        .rev()
        .find(|(_, character)| !is_completion_token(*character))
        .map_or(0, |(index, character)| index + character.len_utf8());
    let token = &line[token_start..];
    if token.is_empty() {
        return Ok(no_completion(line));
    }

    let (table, prefix, candidate_prefix) = if let Some(dot) = token.rfind('.') {
        let path = &token[..dot];
        let Some(table) = resolve_table_path(state, gc, path)? else {
            return Ok(no_completion(line));
        };
        (table, &token[dot + 1..], format!("{path}."))
    } else {
        let Some(table) = state.global_table else {
            return Ok(no_completion(line));
        };
        (table, token, String::new())
    };
    let candidates = collect_string_keys(gc, table, prefix, &candidate_prefix)?;
    Ok(build_result(line, token_start, token, candidates))
}

fn resolve_table_path(
    state: &LuaState,
    gc: &GarbageCollector,
    path: &str,
) -> Result<Option<GcRef<Table>>, String> {
    let mut parts = path.split('.');
    let Some(first) = parts.next().filter(|part| !part.is_empty()) else {
        return Ok(None);
    };
    let Some(global) = state.global_table else {
        return Ok(None);
    };
    let mut current = table_string_value(state, gc, global, first)?;
    for part in parts {
        if part.is_empty() {
            return Ok(None);
        }
        let Value::Table(table) = current else {
            return Ok(None);
        };
        current = table_string_value(state, gc, table, part)?;
    }
    Ok(match current {
        Value::Table(table) => Some(table),
        _ => None,
    })
}

fn table_string_value(
    state: &LuaState,
    gc: &GarbageCollector,
    table: GcRef<Table>,
    key: &str,
) -> Result<Value, String> {
    let Some(key) = state
        .find_interned_string(key.as_bytes())
        .map_err(|error| error.to_string())?
    else {
        return Ok(Value::Nil);
    };
    gc.with_ref(table, |table| table.get(&Value::String(key)))
        .map_err(|error| error.to_string())
}

fn collect_string_keys(
    gc: &GarbageCollector,
    table: GcRef<Table>,
    prefix: &str,
    candidate_prefix: &str,
) -> Result<Vec<String>, String> {
    let keys = gc
        .with_ref(table, |table| {
            table
                .hash_entries()
                .filter_map(|(key, _)| match key {
                    Value::String(key) => Some(*key),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .map_err(|error| error.to_string())?;
    let mut candidates = Vec::new();
    for key in keys {
        let name = gc
            .with_string_bytes(key, |bytes| {
                let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
                String::from_utf8_lossy(bytes).into_owned()
            })
            .map_err(|error| error.to_string())?;
        if name.starts_with(prefix) {
            candidates.push(format!("{candidate_prefix}{name}"));
        }
    }
    candidates.sort();
    candidates.dedup();
    Ok(candidates)
}

fn build_result(
    line: &str,
    token_start: usize,
    token: &str,
    mut candidates: Vec<String>,
) -> CompletionResult {
    candidates.sort();
    candidates.dedup();
    let replacement = match candidates.as_slice() {
        [] => String::new(),
        [candidate] => candidate.clone(),
        _ => common_prefix(&candidates),
    };
    let completed = if replacement.len() > token.len() {
        format!("{}{replacement}", &line[..token_start])
    } else {
        line.to_string()
    };
    CompletionResult {
        line: completed,
        candidates,
    }
}

fn common_prefix(values: &[String]) -> String {
    let mut prefix = values.first().cloned().unwrap_or_default();
    for value in &values[1..] {
        let length = prefix
            .bytes()
            .zip(value.bytes())
            .take_while(|(left, right)| left == right)
            .count();
        prefix.truncate(length);
    }
    prefix
}

fn no_completion(line: &str) -> CompletionResult {
    CompletionResult {
        line: line.to_string(),
        candidates: Vec::new(),
    }
}

fn is_completion_token(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_completion_matches_fixed_candidates_and_common_prefix() {
        let single = complete_meta(".g", 0);
        assert_eq!(single.line, ".gc");
        assert_eq!(single.candidates, [".gc"]);

        let ambiguous = complete_meta(".gc st", 0);
        assert_eq!(ambiguous.line, ".gc st");
        assert_eq!(ambiguous.candidates, ["stats", "status", "strategy"]);
    }
}
