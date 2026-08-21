//! v0.0: a minimal source-loading shell for the teaching interpreter.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

/// A source chunk keeps its display name and original bytes together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChunk {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The only observable result in v0.0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub chunk_name: String,
    pub byte_count: usize,
}

/// Errors that belong to the command-line input boundary.
#[derive(Debug)]
pub enum InputError {
    TooManyArguments,
    Read {
        source_name: String,
        source: io::Error,
    },
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyArguments => write!(formatter, "expected at most one script path"),
            Self::Read {
                source_name,
                source,
            } => {
                write!(formatter, "cannot read {source_name}: {source}")
            }
        }
    }
}

impl Error for InputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::TooManyArguments => None,
        }
    }
}

/// Load one file, or stdin when no path was provided.
// ANCHOR: input_boundary
pub fn load_source(
    args: impl IntoIterator<Item = OsString>,
    mut stdin: impl Read,
) -> Result<SourceChunk, InputError> {
    let mut args = args.into_iter();
    let first = args.next();
    if args.next().is_some() {
        return Err(InputError::TooManyArguments);
    }

    match first {
        Some(path) => {
            let path = PathBuf::from(path);
            let name = path.to_string_lossy().into_owned();
            let bytes = fs::read(&path).map_err(|source| InputError::Read {
                source_name: name.clone(),
                source,
            })?;
            Ok(SourceChunk { name, bytes })
        }
        None => {
            let mut bytes = Vec::new();
            stdin
                .read_to_end(&mut bytes)
                .map_err(|source| InputError::Read {
                    source_name: "stdin".to_owned(),
                    source,
                })?;
            Ok(SourceChunk {
                name: "stdin".to_owned(),
                bytes,
            })
        }
    }
}

/// The stable execution seam that later versions will fill in.
#[must_use]
pub fn run(chunk: &SourceChunk) -> RunSummary {
    RunSummary {
        chunk_name: chunk.name.clone(),
        byte_count: chunk.bytes.len(),
    }
}
// ANCHOR_END: input_boundary

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_bytes_are_preserved() {
        let chunk = load_source(Vec::<OsString>::new(), &b"return 42\0"[..]).unwrap();

        assert_eq!(chunk.name, "stdin");
        assert_eq!(chunk.bytes, b"return 42\0");
        assert_eq!(run(&chunk).byte_count, 10);
    }

    #[test]
    fn extra_arguments_are_rejected() {
        let result = load_source(
            [OsString::from("a.lua"), OsString::from("b.lua")],
            io::empty(),
        );

        assert!(matches!(result, Err(InputError::TooManyArguments)));
    }
}
