//! Process-level Lua fixture runner.
//!
//! The runner deliberately lives outside the product workspace. It treats the
//! Lua command-line applications as black boxes and records their observable
//! process behavior.

use command_group::CommandGroup;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::{Builder as TempDirBuilder, TempDir};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const ARTIFACT_SCHEMA_VERSION: &str = "lua-fixture-differential/v1";
const MAX_CAPTURED_SIDE_EFFECT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    pub schema_version: u32,
    pub fixture_root: String,
    pub inventory: FixtureInventory,
    pub cases: Vec<FixtureCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureInventory {
    pub m0_baseline: usize,
    pub added_differential: usize,
    pub current_total: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureCase {
    pub id: String,
    pub path: String,
    pub kind: FixtureKind,
    pub working_directory: String,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub expected_exit: i32,
    pub timeout_ms: u64,
    pub compare_stdout: bool,
    pub compare_stderr: bool,
    pub compare_side_effects: bool,
    pub side_effects: Vec<String>,
    pub normalizations: Vec<NormalizationRule>,
    pub oracles: Vec<Oracle>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FixtureKind {
    Entry,
    Helper,
    Negative,
    ManualOutput,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Oracle {
    LuaRust,
    LuaCpp,
    Lua51,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationRule {
    pub target: NormalizationTarget,
    pub kind: NormalizationKind,
    pub pattern: String,
    pub replacement: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NormalizationTarget {
    Stdout,
    Stderr,
    Both,
    SideEffectPath,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NormalizationKind {
    Literal,
    Regex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    NonOfficial,
    Differential,
    Official,
    All,
}

impl Suite {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "non-official" => Ok(Self::NonOfficial),
            "differential" => Ok(Self::Differential),
            "official" => Ok(Self::Official),
            "all" => Ok(Self::All),
            _ => Err(format!(
                "invalid suite '{value}'; expected non-official, differential, official, or all"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NonOfficial => "non-official",
            Self::Differential => "differential",
            Self::Official => "official",
            Self::All => "all",
        }
    }

    pub fn includes(self, case: &FixtureCase) -> bool {
        let official = is_official_path(&case.path);
        let differential = is_differential_path(&case.path);
        match self {
            // The historical non-official corpus is the 101 records that
            // predate the dedicated four-case differential lane.
            Self::NonOfficial => !official && !differential,
            Self::Differential => differential,
            Self::Official => official,
            Self::All => true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunConfiguration {
    pub repo_root: PathBuf,
    pub manifest_path: PathBuf,
    pub rust_executable: PathBuf,
    pub cpp_executable: Option<PathBuf>,
    pub artifact_path: PathBuf,
    pub suite: Suite,
    pub case_ids: BTreeSet<String>,
    pub include_helpers: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunArtifact {
    pub schema_version: &'static str,
    pub manifest_schema_version: u32,
    pub fixture_inventory: FixtureInventory,
    pub runner_version: &'static str,
    pub generated_at_unix_ms: u128,
    pub repository_root: String,
    pub manifest_path: String,
    pub suite: String,
    pub engines: EnginePathsArtifact,
    pub summary: RunSummary,
    pub cases: Vec<CaseArtifact>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnginePathsArtifact {
    pub lua_rust: String,
    pub lua_cpp: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RunSummary {
    pub selected: usize,
    pub executed: usize,
    pub helpers_skipped: usize,
    pub matches: usize,
    pub rust_only_expected: usize,
    pub differences: usize,
    pub runner_errors: usize,
    pub timed_out: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseArtifact {
    pub id: String,
    pub path: String,
    pub kind: FixtureKind,
    pub expected_exit: i32,
    pub timeout_ms: u64,
    pub oracles: Vec<Oracle>,
    pub status: CaseStatus,
    pub skip_reason: Option<String>,
    pub lua_rust: Option<ExecutionArtifact>,
    pub lua_cpp: Option<ExecutionArtifact>,
    pub comparison: Option<ComparisonArtifact>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CaseStatus {
    Match,
    RustOnlyExpected,
    Difference,
    RunnerError,
    HelperSkipped,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionArtifact {
    pub engine: &'static str,
    pub executable: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub duration_ms: u128,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub expected_exit_matched: bool,
    pub spawn_error: Option<String>,
    pub stdout: ByteArtifact,
    pub stderr: ByteArtifact,
    pub side_effects: Vec<SideEffectArtifact>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ByteArtifact {
    pub byte_length: usize,
    pub hex: String,
    pub lossy_text: String,
    pub normalized_byte_length: usize,
    pub normalized_hex: String,
    pub normalized_lossy_text: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SideEffectArtifact {
    pub path: String,
    pub normalized_path: String,
    pub byte_length: usize,
    pub content_hex: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComparisonArtifact {
    pub timeout_equal: bool,
    pub exit_code_equal: bool,
    pub stdout_compared: bool,
    pub stdout_equal: Option<bool>,
    pub stderr_compared: bool,
    pub stderr_equal: Option<bool>,
    pub side_effects_compared: bool,
    pub side_effects_equal: Option<bool>,
    pub matched: bool,
    pub stdout_diff: Option<String>,
    pub stderr_diff: Option<String>,
    pub side_effects_diff: Option<String>,
}

#[derive(Debug)]
struct CapturedProcess {
    duration_ms: u128,
    timed_out: bool,
    exit_code: Option<i32>,
    spawn_error: Option<String>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug)]
struct WorkDirectory {
    path: PathBuf,
    temporary: Option<TempDir>,
}

#[derive(Debug)]
struct NormalizationContext<'a> {
    executable: &'a Path,
    working_directory: &'a Path,
    repo_root: &'a Path,
    script_path: &'a Path,
    temp_directory: Option<&'a Path>,
}

pub fn discover_repo_root(start: &Path) -> Result<PathBuf, String> {
    let mut cursor = absolute_path(start)?;
    if cursor.is_file() {
        cursor.pop();
    }

    loop {
        if cursor.join("Cargo.toml").is_file() && cursor.join("tests/lua").is_dir() {
            return Ok(cursor);
        }
        if !cursor.pop() {
            return Err(format!(
                "could not find lua_rust repository root above {}",
                start.display()
            ));
        }
    }
}

pub fn load_and_validate_manifest(
    manifest_path: &Path,
    repo_root: &Path,
) -> Result<FixtureManifest, String> {
    let bytes = fs::read(manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest: FixtureManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
    validate_manifest(&manifest, repo_root)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &FixtureManifest, repo_root: &Path) -> Result<(), String> {
    let mut errors = Vec::new();

    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        errors.push(format!(
            "schema_version is {}, expected {MANIFEST_SCHEMA_VERSION}",
            manifest.schema_version
        ));
    }
    if manifest.inventory.m0_baseline != 125 {
        errors.push(format!(
            "inventory.m0_baseline is {}, expected the audited M0 baseline of 125",
            manifest.inventory.m0_baseline
        ));
    }
    let classified_differential = manifest
        .cases
        .iter()
        .filter(|case| is_differential_path(&case.path))
        .count();
    if manifest.inventory.added_differential != classified_differential {
        errors.push(format!(
            "inventory.added_differential is {}, but {} differential fixtures are classified",
            manifest.inventory.added_differential, classified_differential
        ));
    }
    if manifest.inventory.current_total != manifest.cases.len() {
        errors.push(format!(
            "inventory.current_total is {}, but manifest contains {} cases",
            manifest.inventory.current_total,
            manifest.cases.len()
        ));
    }
    if manifest.inventory.current_total
        != manifest.inventory.m0_baseline + manifest.inventory.added_differential
    {
        errors.push(format!(
            "inventory total {} does not equal baseline {} + differential {}",
            manifest.inventory.current_total,
            manifest.inventory.m0_baseline,
            manifest.inventory.added_differential
        ));
    }

    if let Err(error) = validate_relative_path(&manifest.fixture_root) {
        errors.push(format!("fixture_root: {error}"));
    }

    let fixture_root = repo_root.join(&manifest.fixture_root);
    if !fixture_root.is_dir() {
        errors.push(format!(
            "fixture_root does not exist: {}",
            fixture_root.display()
        ));
    }

    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();

    for (index, case) in manifest.cases.iter().enumerate() {
        let prefix = format!("cases[{index}] ({})", case.id);
        if case.id.trim().is_empty() {
            errors.push(format!("{prefix}: id must not be empty"));
        } else if !ids.insert(case.id.clone()) {
            errors.push(format!("{prefix}: duplicate id"));
        }

        if let Err(error) = validate_relative_path(&case.path) {
            errors.push(format!("{prefix}: path: {error}"));
        } else {
            if !case.path.ends_with(".lua") {
                errors.push(format!("{prefix}: path must end in .lua"));
            }
            if !paths.insert(normalize_relative_path(&case.path)) {
                errors.push(format!("{prefix}: duplicate path"));
            }
            let full_path = repo_root.join(&case.path);
            if !full_path.is_file() {
                errors.push(format!(
                    "{prefix}: fixture does not exist: {}",
                    full_path.display()
                ));
            }
        }

        if case.timeout_ms == 0 {
            errors.push(format!("{prefix}: timeout_ms must be greater than zero"));
        }
        if case.kind == FixtureKind::Negative && case.expected_exit == 0 {
            errors.push(format!(
                "{prefix}: negative fixture must declare a non-zero expected_exit"
            ));
        }
        if case.oracles.is_empty() {
            errors.push(format!("{prefix}: at least one oracle is required"));
        } else {
            let unique_oracles: BTreeSet<_> = case.oracles.iter().copied().collect();
            if unique_oracles.len() != case.oracles.len() {
                errors.push(format!("{prefix}: duplicate oracle"));
            }
            if !unique_oracles.contains(&Oracle::LuaRust) {
                errors.push(format!("{prefix}: lua_rust oracle is required"));
            }
        }

        if let Err(error) = validate_working_directory(&case.working_directory, repo_root) {
            errors.push(format!("{prefix}: working_directory: {error}"));
        }

        for side_effect in &case.side_effects {
            if let Err(error) = validate_relative_path(side_effect) {
                errors.push(format!("{prefix}: side effect '{side_effect}': {error}"));
            }
        }

        for (rule_index, rule) in case.normalizations.iter().enumerate() {
            if rule.pattern.is_empty() {
                errors.push(format!(
                    "{prefix}: normalizations[{rule_index}] pattern must not be empty"
                ));
            }
            if rule.kind == NormalizationKind::Regex {
                let placeholder_pattern = replace_templates_for_validation(&rule.pattern);
                if let Err(error) = Regex::new(&placeholder_pattern) {
                    errors.push(format!(
                        "{prefix}: normalizations[{rule_index}] invalid regex: {error}"
                    ));
                }
            }
        }
    }

    if fixture_root.is_dir() {
        match collect_lua_paths(&fixture_root, repo_root) {
            Ok(actual_paths) => {
                for path in actual_paths.difference(&paths) {
                    errors.push(format!("unclassified Lua fixture: {path}"));
                }
                for path in paths.difference(&actual_paths) {
                    errors.push(format!("manifest path is not a Lua fixture: {path}"));
                }
                if actual_paths.len() != manifest.cases.len() {
                    errors.push(format!(
                        "fixture count mismatch: filesystem={}, manifest={}",
                        actual_paths.len(),
                        manifest.cases.len()
                    ));
                }
            }
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "fixture manifest validation failed:\n- {}",
            errors.join("\n- ")
        ))
    }
}

pub fn run_manifest(manifest: &FixtureManifest, configuration: &RunConfiguration) -> RunArtifact {
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let rust_executable = absolute_path_lossy(&configuration.rust_executable);
    let cpp_executable = configuration
        .cpp_executable
        .as_ref()
        .map(|path| absolute_path_lossy(path));

    let mut artifact = RunArtifact {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        manifest_schema_version: manifest.schema_version,
        fixture_inventory: manifest.inventory.clone(),
        runner_version: env!("CARGO_PKG_VERSION"),
        generated_at_unix_ms,
        repository_root: path_string(&configuration.repo_root),
        manifest_path: path_string(&configuration.manifest_path),
        suite: configuration.suite.as_str().to_owned(),
        engines: EnginePathsArtifact {
            lua_rust: path_string(&rust_executable),
            lua_cpp: cpp_executable.as_ref().map(|path| path_string(path)),
        },
        summary: RunSummary::default(),
        cases: Vec::new(),
    };

    for case in manifest.cases.iter().filter(|case| {
        configuration.suite.includes(case)
            && (configuration.case_ids.is_empty() || configuration.case_ids.contains(&case.id))
    }) {
        artifact.summary.selected += 1;
        let case_artifact = run_case(
            case,
            configuration,
            &rust_executable,
            cpp_executable.as_deref(),
        );

        match case_artifact.status {
            CaseStatus::Match => artifact.summary.matches += 1,
            CaseStatus::RustOnlyExpected => artifact.summary.rust_only_expected += 1,
            CaseStatus::Difference => artifact.summary.differences += 1,
            CaseStatus::RunnerError => artifact.summary.runner_errors += 1,
            CaseStatus::HelperSkipped => artifact.summary.helpers_skipped += 1,
        }
        if case_artifact.status != CaseStatus::HelperSkipped {
            artifact.summary.executed += 1;
        }
        if case_artifact
            .lua_rust
            .iter()
            .chain(case_artifact.lua_cpp.iter())
            .any(|execution| execution.timed_out)
        {
            artifact.summary.timed_out += 1;
        }
        artifact.cases.push(case_artifact);
    }

    artifact
}

pub fn write_artifact(path: &Path, artifact: &RunArtifact) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let file = fs::File::create(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    serde_json::to_writer_pretty(file, artifact)
        .map_err(|error| format!("failed to serialize artifact {}: {error}", path.display()))
}

fn run_case(
    case: &FixtureCase,
    configuration: &RunConfiguration,
    rust_executable: &Path,
    cpp_executable: Option<&Path>,
) -> CaseArtifact {
    if case.kind == FixtureKind::Helper && !configuration.include_helpers {
        return CaseArtifact {
            id: case.id.clone(),
            path: case.path.clone(),
            kind: case.kind,
            expected_exit: case.expected_exit,
            timeout_ms: case.timeout_ms,
            oracles: case.oracles.clone(),
            status: CaseStatus::HelperSkipped,
            skip_reason: Some("classified as helper; use --include-helpers to execute".to_owned()),
            lua_rust: None,
            lua_cpp: None,
            comparison: None,
        };
    }

    let rust_result = execute_case("lua_rust", rust_executable, case, &configuration.repo_root);
    let cpp_result = cpp_executable
        .filter(|_| case.oracles.contains(&Oracle::LuaCpp))
        .map(|executable| execute_case("lua_cpp", executable, case, &configuration.repo_root));

    let has_runner_error = rust_result.spawn_error.is_some()
        || cpp_result
            .as_ref()
            .is_some_and(|execution| execution.spawn_error.is_some());
    let comparison = cpp_result
        .as_ref()
        .map(|cpp| compare_executions(case, &rust_result, cpp));

    let status = if has_runner_error {
        CaseStatus::RunnerError
    } else if let Some(comparison) = &comparison {
        if rust_result.expected_exit_matched
            && cpp_result
                .as_ref()
                .is_some_and(|execution| execution.expected_exit_matched)
            && comparison.matched
        {
            CaseStatus::Match
        } else {
            CaseStatus::Difference
        }
    } else if rust_result.expected_exit_matched {
        CaseStatus::RustOnlyExpected
    } else {
        CaseStatus::Difference
    };

    CaseArtifact {
        id: case.id.clone(),
        path: case.path.clone(),
        kind: case.kind,
        expected_exit: case.expected_exit,
        timeout_ms: case.timeout_ms,
        oracles: case.oracles.clone(),
        status,
        skip_reason: None,
        lua_rust: Some(rust_result),
        lua_cpp: cpp_result,
        comparison,
    }
}

fn execute_case(
    engine: &'static str,
    executable: &Path,
    case: &FixtureCase,
    repo_root: &Path,
) -> ExecutionArtifact {
    let script_path = absolute_path_lossy(&repo_root.join(&case.path));
    let work_directory = prepare_work_directory(case, repo_root);

    let (working_directory, temporary_path, work_error) = match &work_directory {
        Ok(work) => (
            work.path.clone(),
            work.temporary.as_ref().map(TempDir::path),
            None,
        ),
        Err(error) => (repo_root.to_path_buf(), None, Some(error.clone())),
    };

    let argv = std::iter::once(path_string(&script_path))
        .chain(case.args.iter().cloned())
        .collect::<Vec<_>>();
    let mut captured = if let Some(error) = work_error {
        CapturedProcess {
            duration_ms: 0,
            timed_out: false,
            exit_code: None,
            spawn_error: Some(error),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    } else {
        capture_process(
            executable,
            &script_path,
            &case.args,
            &case.environment,
            &working_directory,
            case.timeout_ms,
        )
    };

    let context = NormalizationContext {
        executable,
        working_directory: &working_directory,
        repo_root,
        script_path: &script_path,
        temp_directory: temporary_path,
    };
    let normalized_stdout = normalize_bytes(
        &captured.stdout,
        &case.normalizations,
        NormalizationTarget::Stdout,
        &context,
    );
    let normalized_stderr = normalize_bytes(
        &captured.stderr,
        &case.normalizations,
        NormalizationTarget::Stderr,
        &context,
    );
    let side_effects = if let Ok(work) = &work_directory {
        match capture_side_effects(work, case, &context) {
            Ok(side_effects) => side_effects,
            Err(error) => {
                append_error(&mut captured.spawn_error, error);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    ExecutionArtifact {
        engine,
        executable: path_string(executable),
        argv,
        working_directory: path_string(&working_directory),
        duration_ms: captured.duration_ms,
        timed_out: captured.timed_out,
        exit_code: captured.exit_code,
        expected_exit_matched: !captured.timed_out
            && captured.spawn_error.is_none()
            && captured.exit_code == Some(case.expected_exit),
        spawn_error: captured.spawn_error,
        stdout: byte_artifact(&captured.stdout, &normalized_stdout),
        stderr: byte_artifact(&captured.stderr, &normalized_stderr),
        side_effects,
    }
}

fn prepare_work_directory(case: &FixtureCase, repo_root: &Path) -> Result<WorkDirectory, String> {
    match case.working_directory.as_str() {
        "temporary" => {
            let temporary = TempDirBuilder::new()
                .prefix(&format!("lua-fixture-{}-", sanitize_id(&case.id)))
                .tempdir()
                .map_err(|error| {
                    format!("failed to create temporary working directory: {error}")
                })?;
            Ok(WorkDirectory {
                path: temporary.path().to_path_buf(),
                temporary: Some(temporary),
            })
        }
        "repository" => Ok(WorkDirectory {
            path: repo_root.to_path_buf(),
            temporary: None,
        }),
        "script-directory" => {
            let path = repo_root.join(&case.path);
            let parent = path.parent().ok_or_else(|| {
                format!("fixture path has no parent directory: {}", path.display())
            })?;
            Ok(WorkDirectory {
                path: parent.to_path_buf(),
                temporary: None,
            })
        }
        relative => Ok(WorkDirectory {
            path: repo_root.join(relative),
            temporary: None,
        }),
    }
}

fn capture_process(
    executable: &Path,
    script_path: &Path,
    args: &[String],
    environment: &BTreeMap<String, String>,
    working_directory: &Path,
    timeout_ms: u64,
) -> CapturedProcess {
    let started = Instant::now();
    let mut command = Command::new(executable);
    command
        .arg(script_path)
        .args(args)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }

    let mut child = match command.group_spawn() {
        Ok(child) => child,
        Err(error) => {
            return CapturedProcess {
                duration_ms: started.elapsed().as_millis(),
                timed_out: false,
                exit_code: None,
                spawn_error: Some(format!("failed to spawn {}: {error}", executable.display())),
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
        }
    };

    let stdout_reader = child.inner().stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        })
    });
    let stderr_reader = child.inner().stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        })
    });

    let deadline = started + Duration::from_millis(timeout_ms);
    let (timed_out, status, mut spawn_error) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (false, Some(status), None),
            Ok(None) if Instant::now() >= deadline => {
                let kill_error = child.kill().err();
                let status = child.wait().ok();
                break (
                    true,
                    status,
                    kill_error.map(|error| {
                        format!("timed out and failed to kill child process group: {error}")
                    }),
                );
            }
            Ok(None) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(error) => {
                let _ = child.kill();
                let status = child.wait().ok();
                break (
                    false,
                    status,
                    Some(format!(
                        "failed while waiting for child process group: {error}"
                    )),
                );
            }
        }
    };

    let stdout = join_reader(stdout_reader, "stdout", &mut spawn_error);
    let stderr = join_reader(stderr_reader, "stderr", &mut spawn_error);

    CapturedProcess {
        duration_ms: started.elapsed().as_millis(),
        timed_out,
        exit_code: status.and_then(|status| status.code()),
        spawn_error,
        stdout,
        stderr,
    }
}

fn join_reader(
    reader: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
    stream: &str,
    process_error: &mut Option<String>,
) -> Vec<u8> {
    let Some(reader) = reader else {
        return Vec::new();
    };
    match reader.join() {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            append_error(
                process_error,
                format!("failed to capture child {stream}: {error}"),
            );
            Vec::new()
        }
        Err(_) => {
            append_error(
                process_error,
                format!("child {stream} capture thread panicked"),
            );
            Vec::new()
        }
    }
}

fn append_error(target: &mut Option<String>, error: String) {
    match target {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        None => *target = Some(error),
    }
}

fn capture_side_effects(
    work: &WorkDirectory,
    case: &FixtureCase,
    context: &NormalizationContext<'_>,
) -> Result<Vec<SideEffectArtifact>, String> {
    let paths = if work.temporary.is_some() {
        collect_files_recursively(&work.path)?
    } else {
        let mut paths = Vec::new();
        for relative in &case.side_effects {
            let path = work.path.join(relative);
            if path.is_file() {
                paths.push(path);
            }
        }
        paths
    };

    let mut artifacts = Vec::new();
    for path in paths {
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.len() > MAX_CAPTURED_SIDE_EFFECT_BYTES {
            return Err(format!(
                "side effect {} is {} bytes, above the {} byte capture limit",
                path.display(),
                metadata.len(),
                MAX_CAPTURED_SIDE_EFFECT_BYTES
            ));
        }
        let content = fs::read(&path)
            .map_err(|error| format!("failed to read side effect {}: {error}", path.display()))?;
        let relative = path
            .strip_prefix(&work.path)
            .map_err(|error| format!("failed to relativize {}: {error}", path.display()))?;
        let relative_text = normalize_relative_path(&relative.to_string_lossy());
        let normalized_path = normalize_text(
            &relative_text,
            &case.normalizations,
            NormalizationTarget::SideEffectPath,
            context,
        );
        artifacts.push(SideEffectArtifact {
            path: relative_text,
            normalized_path,
            byte_length: content.len(),
            content_hex: hex_encode(&content),
        });
    }
    artifacts.sort_by(|left, right| {
        left.normalized_path
            .cmp(&right.normalized_path)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(artifacts)
}

fn compare_executions(
    case: &FixtureCase,
    rust: &ExecutionArtifact,
    cpp: &ExecutionArtifact,
) -> ComparisonArtifact {
    let timeout_equal = rust.timed_out == cpp.timed_out;
    let exit_code_equal = rust.exit_code == cpp.exit_code;
    let stdout_equal = case
        .compare_stdout
        .then(|| rust.stdout.normalized_hex == cpp.stdout.normalized_hex);
    let stderr_equal = case
        .compare_stderr
        .then(|| rust.stderr.normalized_hex == cpp.stderr.normalized_hex);
    let side_effects_equal = case.compare_side_effects.then(|| {
        normalized_side_effects(&rust.side_effects) == normalized_side_effects(&cpp.side_effects)
    });
    let matched = timeout_equal
        && exit_code_equal
        && stdout_equal.unwrap_or(true)
        && stderr_equal.unwrap_or(true)
        && side_effects_equal.unwrap_or(true);

    ComparisonArtifact {
        timeout_equal,
        exit_code_equal,
        stdout_compared: case.compare_stdout,
        stdout_equal,
        stderr_compared: case.compare_stderr,
        stderr_equal,
        side_effects_compared: case.compare_side_effects,
        side_effects_equal,
        matched,
        stdout_diff: case
            .compare_stdout
            .then(|| {
                describe_byte_difference(
                    rust.stdout.normalized_hex.as_bytes(),
                    cpp.stdout.normalized_hex.as_bytes(),
                    &rust.stdout.normalized_lossy_text,
                    &cpp.stdout.normalized_lossy_text,
                )
            })
            .filter(|_| stdout_equal == Some(false)),
        stderr_diff: case
            .compare_stderr
            .then(|| {
                describe_byte_difference(
                    rust.stderr.normalized_hex.as_bytes(),
                    cpp.stderr.normalized_hex.as_bytes(),
                    &rust.stderr.normalized_lossy_text,
                    &cpp.stderr.normalized_lossy_text,
                )
            })
            .filter(|_| stderr_equal == Some(false)),
        side_effects_diff: side_effects_equal
            .filter(|equal| !equal)
            .map(|_| describe_side_effect_difference(&rust.side_effects, &cpp.side_effects)),
    }
}

fn normalized_side_effects(side_effects: &[SideEffectArtifact]) -> Vec<(&str, &str)> {
    side_effects
        .iter()
        .map(|effect| (effect.normalized_path.as_str(), effect.content_hex.as_str()))
        .collect()
}

fn describe_byte_difference(
    rust_hex: &[u8],
    cpp_hex: &[u8],
    rust_text: &str,
    cpp_text: &str,
) -> String {
    let first_hex_difference = rust_hex
        .iter()
        .zip(cpp_hex)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| rust_hex.len().min(cpp_hex.len()));
    let byte_offset = first_hex_difference / 2;
    let (rust_line, cpp_line) = first_different_line(rust_text, cpp_text);
    let context_start = first_hex_difference.saturating_sub(16) & !1;
    let rust_context_end = (first_hex_difference + 18).min(rust_hex.len());
    let cpp_context_end = (first_hex_difference + 18).min(cpp_hex.len());
    let rust_context = String::from_utf8_lossy(&rust_hex[context_start..rust_context_end]);
    let cpp_context = String::from_utf8_lossy(&cpp_hex[context_start..cpp_context_end]);
    format!(
        "first differing byte offset {byte_offset}; rust hex={rust_context}; cpp hex={cpp_context}; rust line={rust_line:?}; cpp line={cpp_line:?}"
    )
}

fn first_different_line<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    let mut left_lines = left.lines();
    let mut right_lines = right.lines();
    loop {
        match (left_lines.next(), right_lines.next()) {
            (Some(left_line), Some(right_line)) if left_line == right_line => {}
            (Some(left_line), Some(right_line)) => return (left_line, right_line),
            (Some(left_line), None) => return (left_line, "<end-of-stream>"),
            (None, Some(right_line)) => return ("<end-of-stream>", right_line),
            (None, None) => return ("<identical-text>", "<identical-text>"),
        }
    }
}

fn describe_side_effect_difference(
    rust: &[SideEffectArtifact],
    cpp: &[SideEffectArtifact],
) -> String {
    let rust_paths = rust
        .iter()
        .map(|effect| effect.normalized_path.as_str())
        .collect::<Vec<_>>();
    let cpp_paths = cpp
        .iter()
        .map(|effect| effect.normalized_path.as_str())
        .collect::<Vec<_>>();
    let content_difference = rust.iter().find_map(|rust_effect| {
        cpp.iter()
            .find(|cpp_effect| cpp_effect.normalized_path == rust_effect.normalized_path)
            .filter(|cpp_effect| cpp_effect.content_hex != rust_effect.content_hex)
            .map(|cpp_effect| {
                let first_hex_difference = rust_effect
                    .content_hex
                    .as_bytes()
                    .iter()
                    .zip(cpp_effect.content_hex.as_bytes())
                    .position(|(left, right)| left != right)
                    .unwrap_or_else(|| {
                        rust_effect
                            .content_hex
                            .len()
                            .min(cpp_effect.content_hex.len())
                    });
                format!(
                    "first content difference: {} at byte {}",
                    rust_effect.normalized_path,
                    first_hex_difference / 2
                )
            })
    });
    format!(
        "rust files={rust_paths:?}; cpp files={cpp_paths:?}; {}",
        content_difference.unwrap_or_else(|| "file sets differ".to_owned())
    )
}

fn byte_artifact(raw: &[u8], normalized: &[u8]) -> ByteArtifact {
    ByteArtifact {
        byte_length: raw.len(),
        hex: hex_encode(raw),
        lossy_text: String::from_utf8_lossy(raw).into_owned(),
        normalized_byte_length: normalized.len(),
        normalized_hex: hex_encode(normalized),
        normalized_lossy_text: String::from_utf8_lossy(normalized).into_owned(),
    }
}

fn normalize_bytes(
    bytes: &[u8],
    rules: &[NormalizationRule],
    target: NormalizationTarget,
    context: &NormalizationContext<'_>,
) -> Vec<u8> {
    let mut result = bytes.to_vec();
    for rule in rules.iter().filter(|rule| {
        rule.target == target
            || (rule.target == NormalizationTarget::Both
                && target != NormalizationTarget::SideEffectPath)
    }) {
        result = match rule.kind {
            NormalizationKind::Literal => {
                let pattern = expand_template(&rule.pattern, context, false);
                replace_bytes(&result, pattern.as_bytes(), rule.replacement.as_bytes())
            }
            NormalizationKind::Regex => {
                let Ok(text) = std::str::from_utf8(&result) else {
                    continue;
                };
                let pattern = expand_template(&rule.pattern, context, true);
                let Ok(regex) = Regex::new(&pattern) else {
                    continue;
                };
                regex
                    .replace_all(text, rule.replacement.as_str())
                    .into_owned()
                    .into_bytes()
            }
        };
    }
    result
}

fn normalize_text(
    text: &str,
    rules: &[NormalizationRule],
    target: NormalizationTarget,
    context: &NormalizationContext<'_>,
) -> String {
    String::from_utf8_lossy(&normalize_bytes(text.as_bytes(), rules, target, context)).into_owned()
}

fn expand_template(
    template: &str,
    context: &NormalizationContext<'_>,
    regex_escape: bool,
) -> String {
    let replacements = [
        ("{{executable}}", path_string(context.executable)),
        (
            "{{executable_slash}}",
            path_string_with_forward_slashes(context.executable),
        ),
        (
            "{{working_directory}}",
            path_string(context.working_directory),
        ),
        (
            "{{working_directory_slash}}",
            path_string_with_forward_slashes(context.working_directory),
        ),
        ("{{repo_root}}", path_string(context.repo_root)),
        (
            "{{repo_root_slash}}",
            path_string_with_forward_slashes(context.repo_root),
        ),
        ("{{script_path}}", path_string(context.script_path)),
        (
            "{{script_path_slash}}",
            path_string_with_forward_slashes(context.script_path),
        ),
        (
            "{{temp_directory}}",
            context.temp_directory.map(path_string).unwrap_or_default(),
        ),
        (
            "{{temp_directory_slash}}",
            context
                .temp_directory
                .map(path_string_with_forward_slashes)
                .unwrap_or_default(),
        ),
    ];
    let mut expanded = template.to_owned();
    for (placeholder, value) in replacements {
        let value = if regex_escape {
            regex::escape(&value)
        } else {
            value
        };
        expanded = expanded.replace(placeholder, &value);
    }
    expanded
}

fn replace_bytes(input: &[u8], pattern: &[u8], replacement: &[u8]) -> Vec<u8> {
    if pattern.is_empty() {
        return input.to_vec();
    }
    let mut result = Vec::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        if input[cursor..].starts_with(pattern) {
            result.extend_from_slice(replacement);
            cursor += pattern.len();
        } else {
            result.push(input[cursor]);
            cursor += 1;
        }
    }
    result
}

fn collect_lua_paths(root: &Path, repo_root: &Path) -> Result<BTreeSet<String>, String> {
    collect_files_recursively(root)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "lua"))
        .map(|path| {
            path.strip_prefix(repo_root)
                .map(|relative| normalize_relative_path(&relative.to_string_lossy()))
                .map_err(|error| format!("failed to relativize {}: {error}", path.display()))
        })
        .collect::<Result<_, _>>()
}

fn collect_files_recursively(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("failed to read {}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("failed to read entry in {}: {error}", directory.display())
            })?;
            let file_type = entry.file_type().map_err(|error| {
                format!("failed to inspect {}: {error}", entry.path().display())
            })?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn validate_working_directory(value: &str, repo_root: &Path) -> Result<(), String> {
    match value {
        "temporary" | "repository" | "script-directory" => Ok(()),
        relative => {
            validate_relative_path(relative)?;
            let path = repo_root.join(relative);
            if path.is_dir() {
                Ok(())
            } else {
                Err(format!("directory does not exist: {}", path.display()))
            }
        }
    }
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("must not be empty".to_owned());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err("must be relative".to_owned());
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("must not contain parent/root/prefix components".to_owned());
    }
    Ok(())
}

fn replace_templates_for_validation(pattern: &str) -> String {
    [
        "{{executable}}",
        "{{executable_slash}}",
        "{{working_directory}}",
        "{{working_directory_slash}}",
        "{{repo_root}}",
        "{{repo_root_slash}}",
        "{{script_path}}",
        "{{script_path_slash}}",
        "{{temp_directory}}",
        "{{temp_directory_slash}}",
    ]
    .iter()
    .fold(pattern.to_owned(), |value, placeholder| {
        value.replace(placeholder, "placeholder")
    })
}

fn is_official_path(path: &str) -> bool {
    normalize_relative_path(path).starts_with("tests/lua/official/")
}

fn is_differential_path(path: &str) -> bool {
    normalize_relative_path(path).starts_with("tests/lua/differential/")
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn normalize_relative_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("failed to read current directory: {error}"))
    }
}

fn absolute_path_lossy(path: &Path) -> PathBuf {
    absolute_path(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn path_string_with_forward_slashes(path: &Path) -> String {
    path_string(path).replace('\\', "/")
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn find_default_rust_executable(repo_root: &Path) -> Option<PathBuf> {
    let executable_name = executable_name("lua_app");
    [
        repo_root.join("target/debug").join(&executable_name),
        repo_root
            .join("target/x86_64-pc-windows-msvc/debug")
            .join(&executable_name),
        repo_root.join("target/release").join(&executable_name),
        repo_root
            .join("target/x86_64-pc-windows-msvc/release")
            .join(&executable_name),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

pub fn find_default_cpp_executable(repo_root: &Path) -> Option<PathBuf> {
    let executable_name = executable_name("lua_app");
    repo_root
        .parent()
        .map(|parent| parent.join("lua_cpp/bin").join(executable_name))
        .filter(|path| path.is_file())
}

fn executable_name(stem: &str) -> OsString {
    if cfg!(windows) {
        format!("{stem}.exe").into()
    } else {
        stem.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_normalization_is_byte_preserving() {
        let executable = PathBuf::from("/tmp/lua app");
        let working_directory = PathBuf::from("/tmp/work");
        let script_path = working_directory.join("test.lua");
        let context = NormalizationContext {
            executable: &executable,
            working_directory: &working_directory,
            repo_root: &working_directory,
            script_path: &script_path,
            temp_directory: Some(&working_directory),
        };
        let rules = vec![NormalizationRule {
            target: NormalizationTarget::Stdout,
            kind: NormalizationKind::Literal,
            pattern: "{{executable}}".to_owned(),
            replacement: "<lua>".to_owned(),
        }];
        let input = b"prefix /tmp/lua app suffix \xff";
        let output = normalize_bytes(input, &rules, NormalizationTarget::Stdout, &context);
        assert_eq!(output, b"prefix <lua> suffix \xff");
    }

    #[test]
    fn forward_slash_path_template_is_explicit() {
        let executable = PathBuf::from(r"C:\runtime\lua_app.exe");
        let working_directory = PathBuf::from(r"C:\work");
        let script_path = working_directory.join("test.lua");
        let context = NormalizationContext {
            executable: &executable,
            working_directory: &working_directory,
            repo_root: &working_directory,
            script_path: &script_path,
            temp_directory: Some(&working_directory),
        };
        let rules = vec![NormalizationRule {
            target: NormalizationTarget::Both,
            kind: NormalizationKind::Literal,
            pattern: "{{executable_slash}}".to_owned(),
            replacement: "<lua>".to_owned(),
        }];
        let output = normalize_bytes(
            b"C:/runtime/lua_app.exe",
            &rules,
            NormalizationTarget::Stdout,
            &context,
        );
        assert_eq!(output, b"<lua>");
    }

    #[test]
    fn regex_normalization_is_explicit_and_targeted() {
        let executable = PathBuf::from("/tmp/lua");
        let working_directory = PathBuf::from("/tmp/work");
        let script_path = working_directory.join("test.lua");
        let context = NormalizationContext {
            executable: &executable,
            working_directory: &working_directory,
            repo_root: &working_directory,
            script_path: &script_path,
            temp_directory: Some(&working_directory),
        };
        let rules = vec![NormalizationRule {
            target: NormalizationTarget::Stderr,
            kind: NormalizationKind::Regex,
            pattern: r"0x[0-9a-fA-F]+".to_owned(),
            replacement: "<address>".to_owned(),
        }];
        assert_eq!(
            normalize_bytes(
                b"value=0x123abc",
                &rules,
                NormalizationTarget::Stderr,
                &context
            ),
            b"value=<address>"
        );
        assert_eq!(
            normalize_bytes(
                b"value=0x123abc",
                &rules,
                NormalizationTarget::Stdout,
                &context
            ),
            b"value=0x123abc"
        );
    }

    #[test]
    fn process_capture_separates_streams_and_exit_code() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        #[cfg(windows)]
        let (program, args) = (
            PathBuf::from("cmd.exe"),
            vec![
                "/D".to_owned(),
                "/S".to_owned(),
                "/C".to_owned(),
                "(echo stdout)&(echo stderr 1>&2)&exit /b 7".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            PathBuf::from("/bin/sh"),
            vec![
                "-c".to_owned(),
                "printf stdout; printf stderr >&2; exit 7".to_owned(),
            ],
        );
        let script = PathBuf::from(&args[0]);
        let capture = capture_process(
            &program,
            &script,
            &args[1..],
            &BTreeMap::new(),
            temporary.path(),
            5_000,
        );
        assert_eq!(capture.exit_code, Some(7));
        #[cfg(windows)]
        assert_eq!(capture.stdout, b"stdout\r\n");
        #[cfg(not(windows))]
        assert_eq!(capture.stdout, b"stdout");
        #[cfg(windows)]
        assert_eq!(capture.stderr, b"stderr \r\n");
        #[cfg(not(windows))]
        assert_eq!(capture.stderr, b"stderr");
        assert!(!capture.timed_out);
        assert!(capture.spawn_error.is_none(), "{:?}", capture.spawn_error);
    }

    #[test]
    fn process_capture_enforces_timeout() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        #[cfg(windows)]
        let (program, args) = (
            PathBuf::from("cmd.exe"),
            vec![
                "/D".to_owned(),
                "/S".to_owned(),
                "/C".to_owned(),
                "ping -n 6 127.0.0.1 >nul".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            PathBuf::from("/bin/sh"),
            vec!["-c".to_owned(), "sleep 5".to_owned()],
        );
        let script = PathBuf::from(&args[0]);
        let capture = capture_process(
            &program,
            &script,
            &args[1..],
            &BTreeMap::new(),
            temporary.path(),
            50,
        );
        assert!(capture.timed_out);
        assert!(capture.duration_ms < 2_000, "{}", capture.duration_ms);
    }

    #[test]
    fn repository_manifest_classifies_every_lua_file() {
        let repo_root =
            discover_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("repository root");
        let manifest_path = repo_root.join("tests/compatibility/lua_fixtures.json");
        let manifest =
            load_and_validate_manifest(&manifest_path, &repo_root).expect("valid manifest");
        assert_eq!(manifest.cases.len(), 131);
        assert_eq!(
            manifest
                .cases
                .iter()
                .filter(|case| is_differential_path(&case.path))
                .count(),
            6
        );
        assert_eq!(
            manifest
                .cases
                .iter()
                .filter(|case| {
                    !is_official_path(&case.path) && !is_differential_path(&case.path)
                })
                .count(),
            101
        );
    }

    #[test]
    fn manifest_validation_rejects_an_unclassified_fixture() {
        let repo_root =
            discover_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("repository root");
        let manifest_path = repo_root.join("tests/compatibility/lua_fixtures.json");
        let mut manifest =
            load_and_validate_manifest(&manifest_path, &repo_root).expect("valid manifest");
        let removed = manifest.cases.pop().expect("at least one fixture");
        manifest.inventory.current_total = manifest.cases.len();
        let error = validate_manifest(&manifest, &repo_root).expect_err("missing case must fail");
        assert!(
            error.contains(&format!("unclassified Lua fixture: {}", removed.path)),
            "{error}"
        );
    }

    #[test]
    fn manifest_validation_rejects_a_stale_differential_inventory_count() {
        let repo_root =
            discover_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("repository root");
        let manifest_path = repo_root.join("tests/compatibility/lua_fixtures.json");
        let mut manifest =
            load_and_validate_manifest(&manifest_path, &repo_root).expect("valid manifest");
        manifest.inventory.added_differential -= 1;

        let error =
            validate_manifest(&manifest, &repo_root).expect_err("stale inventory must fail");
        assert!(
            error.contains("but 6 differential fixtures are classified"),
            "{error}"
        );
    }

    #[test]
    fn negative_fixture_requires_nonzero_expected_exit() {
        let repo_root =
            discover_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("repository root");
        let manifest_path = repo_root.join("tests/compatibility/lua_fixtures.json");
        let mut manifest =
            load_and_validate_manifest(&manifest_path, &repo_root).expect("valid manifest");
        let negative = manifest
            .cases
            .iter_mut()
            .find(|case| case.kind == FixtureKind::Negative)
            .expect("negative fixture");
        negative.expected_exit = 0;
        let error = validate_manifest(&manifest, &repo_root).expect_err("invalid negative case");
        assert!(
            error.contains("negative fixture must declare a non-zero expected_exit"),
            "{error}"
        );
    }
}
