use lua_fixture_runner::{
    RunConfiguration, Suite, discover_repo_root, find_default_cpp_executable,
    find_default_rust_executable, load_and_validate_manifest, run_manifest, write_artifact,
};
use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Cli {
    repo_root: Option<PathBuf>,
    manifest: Option<PathBuf>,
    rust: Option<PathBuf>,
    cpp: Option<PathBuf>,
    artifact: Option<PathBuf>,
    suite: Suite,
    case_ids: BTreeSet<String>,
    include_helpers: bool,
    rust_only: bool,
    validate_only: bool,
    allow_differences: bool,
}

fn main() {
    let status = match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("lua-fixture-runner: {error}");
            2
        }
    };
    std::process::exit(status);
}

fn run() -> Result<i32, String> {
    let cli = parse_cli(env::args().skip(1))?;
    let current_directory =
        env::current_dir().map_err(|error| format!("failed to read current directory: {error}"))?;
    let repo_root = match cli.repo_root {
        Some(path) => absolute_from(&current_directory, &path),
        None => discover_repo_root(&current_directory)?,
    };
    let manifest_path = cli
        .manifest
        .map(|path| absolute_from(&current_directory, &path))
        .unwrap_or_else(|| repo_root.join("tests/compatibility/lua_fixtures.json"));
    let manifest = load_and_validate_manifest(&manifest_path, &repo_root)?;
    for case_id in &cli.case_ids {
        let case = manifest
            .cases
            .iter()
            .find(|case| &case.id == case_id)
            .ok_or_else(|| format!("unknown --case '{case_id}'"))?;
        if !cli.suite.includes(case) {
            return Err(format!(
                "--case '{}' is not in the '{}' suite",
                case_id,
                cli.suite.as_str()
            ));
        }
    }

    if cli.validate_only {
        println!(
            "manifest valid: {} fixtures (M0 baseline {}, +{} differential; {} non-official, {} official)",
            manifest.cases.len(),
            manifest.inventory.m0_baseline,
            manifest.inventory.added_differential,
            manifest
                .cases
                .iter()
                .filter(|case| {
                    let path = case.path.replace('\\', "/");
                    !path.starts_with("tests/lua/official/")
                        && !path.starts_with("tests/lua/differential/")
                })
                .count(),
            manifest
                .cases
                .iter()
                .filter(|case| case
                    .path
                    .replace('\\', "/")
                    .starts_with("tests/lua/official/"))
                .count()
        );
        return Ok(0);
    }

    let rust_executable = cli
        .rust
        .or_else(|| env::var_os("LUA_RUST_APP").map(PathBuf::from))
        .map(|path| absolute_from(&current_directory, &path))
        .or_else(|| find_default_rust_executable(&repo_root))
        .ok_or_else(|| {
            "could not find lua_rust executable; build lua_app or pass --rust <path>".to_owned()
        })?;
    ensure_file("--rust", &rust_executable)?;

    let cpp_executable = if cli.rust_only {
        None
    } else {
        let path = cli
            .cpp
            .or_else(|| env::var_os("LUA_CPP_APP").map(PathBuf::from))
            .map(|path| absolute_from(&current_directory, &path))
            .or_else(|| find_default_cpp_executable(&repo_root))
            .ok_or_else(|| {
                "could not find lua_cpp executable; pass --cpp <path> or use --rust-only".to_owned()
            })?;
        ensure_file("--cpp", &path)?;
        Some(path)
    };

    let artifact_path = cli
        .artifact
        .map(|path| absolute_from(&current_directory, &path))
        .unwrap_or_else(|| {
            repo_root.join(format!(
                "target/compatibility/lua-fixtures-{}.json",
                cli.suite.as_str()
            ))
        });
    let configuration = RunConfiguration {
        repo_root,
        manifest_path,
        rust_executable,
        cpp_executable,
        artifact_path: artifact_path.clone(),
        suite: cli.suite,
        case_ids: cli.case_ids,
        include_helpers: cli.include_helpers,
    };
    let artifact = run_manifest(&manifest, &configuration);
    write_artifact(&artifact_path, &artifact)?;

    println!(
        "artifact: {}\nselected={} executed={} matched={} rust-only-expected={} differences={} runner-errors={} helpers-skipped={} timed-out={}",
        artifact_path.display(),
        artifact.summary.selected,
        artifact.summary.executed,
        artifact.summary.matches,
        artifact.summary.rust_only_expected,
        artifact.summary.differences,
        artifact.summary.runner_errors,
        artifact.summary.helpers_skipped,
        artifact.summary.timed_out
    );

    if artifact.summary.runner_errors > 0 || artifact.summary.timed_out > 0 {
        Ok(2)
    } else if artifact.summary.differences > 0 && !cli.allow_differences {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn parse_cli(arguments: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli {
        repo_root: None,
        manifest: None,
        rust: None,
        cpp: None,
        artifact: None,
        suite: Suite::NonOfficial,
        case_ids: BTreeSet::new(),
        include_helpers: false,
        rust_only: false,
        validate_only: false,
        allow_differences: false,
    };
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--repo-root" => cli.repo_root = Some(next_path(&mut arguments, "--repo-root")?),
            "--manifest" => cli.manifest = Some(next_path(&mut arguments, "--manifest")?),
            "--rust" => cli.rust = Some(next_path(&mut arguments, "--rust")?),
            "--cpp" => cli.cpp = Some(next_path(&mut arguments, "--cpp")?),
            "--artifact" => cli.artifact = Some(next_path(&mut arguments, "--artifact")?),
            "--suite" => {
                let value = next_value(&mut arguments, "--suite")?;
                cli.suite = Suite::parse(&value)?;
            }
            "--case" => {
                let value = next_value(&mut arguments, "--case")?;
                if !cli.case_ids.insert(value.clone()) {
                    return Err(format!("duplicate --case '{value}'"));
                }
            }
            "--include-helpers" => cli.include_helpers = true,
            "--rust-only" => cli.rust_only = true,
            "--validate-only" => cli.validate_only = true,
            "--allow-differences" => cli.allow_differences = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument '{unknown}'; use --help")),
        }
    }
    if cli.rust_only && cli.cpp.is_some() {
        return Err("--rust-only cannot be combined with --cpp".to_owned());
    }
    if cli.validate_only
        && (cli.rust.is_some()
            || cli.cpp.is_some()
            || cli.artifact.is_some()
            || cli.include_helpers
            || cli.rust_only
            || cli.allow_differences
            || !cli.case_ids.is_empty())
    {
        return Err("--validate-only only accepts --repo-root, --manifest, and --suite".to_owned());
    }
    Ok(cli)
}

fn next_path(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<PathBuf, String> {
    next_value(arguments, option).map(PathBuf::from)
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn absolute_from(current_directory: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_directory.join(path)
    }
}

fn ensure_file(option: &str, path: &Path) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!(
            "{option} executable does not exist: {}",
            path.display()
        ))
    }
}

fn print_help() {
    println!(
        "\
Process-level differential runner for tests/lua.

Usage:
  lua-fixture-runner [options]

Options:
  --repo-root <path>       lua_rust repository root (auto-detected)
  --manifest <path>        fixture manifest (tests/compatibility/lua_fixtures.json)
  --rust <path>            lua_rust CLI executable (or LUA_RUST_APP)
  --cpp <path>             lua_cpp CLI executable (or LUA_CPP_APP)
  --artifact <path>        JSON output path
  --suite <name>           non-official (default), differential, official, or all
  --case <id>              run one exact case ID; repeatable
  --include-helpers        intentionally execute helper/module records
  --rust-only              validate only lua_rust expected exits
  --validate-only          validate schema and complete fixture classification
  --allow-differences      return success when semantic differences are reported
  -h, --help               show this help"
    );
}
