//! Host wall-time and managed-memory observations, not automatic GC pause/RSS.

use std::env;
use std::time::Instant;

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::value::Value;
use lua_vm::Runtime;
use serde_json::{Value as Json, json};

const WORKLOADS: &[(&str, &str, bool)] = &[
    (
        "table-reinsert",
        include_str!("workloads/table_churn.lua"),
        false,
    ),
    (
        "table-rotate",
        include_str!("workloads/table_churn.lua"),
        true,
    ),
    (
        "closure-coroutine",
        include_str!("workloads/closure_coroutine.lua"),
        false,
    ),
    (
        "allocation-pressure",
        include_str!("workloads/allocation_pressure.lua"),
        false,
    ),
];

struct Config {
    iterations: usize,
    width: usize,
    batches: usize,
    warmup: usize,
    workload: String,
}

impl Config {
    fn parse() -> Result<Self, String> {
        let mut config = Self {
            iterations: 1_000,
            width: 128,
            batches: 20,
            warmup: 3,
            workload: "all".into(),
        };
        let mut arguments = env::args().skip(1);
        while let Some(flag) = arguments.next() {
            if flag == "--help" {
                println!(
                    "runtime_baseline [--iterations N] [--width N] [--batches N] [--warmup N] [--workload all|table-reinsert|table-rotate|closure-coroutine|allocation-pressure]\nJSON is written to stdout. Each workload uses one Runtime across all its batches."
                );
                std::process::exit(0);
            }
            let value = arguments
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            if flag == "--workload" {
                config.workload = value;
                continue;
            }
            let number = value
                .parse::<usize>()
                .map_err(|_| format!("invalid integer for {flag}"))?;
            match flag.as_str() {
                "--iterations" => config.iterations = number,
                "--width" => config.width = number,
                "--batches" => config.batches = number,
                "--warmup" => config.warmup = number,
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        if config.iterations == 0 || config.width == 0 || config.batches == 0 {
            return Err("iterations, width, and batches must be positive".into());
        }
        // Keep generated Lua integer/checksum arithmetic exact in binary64.
        let total_batches = config
            .batches
            .checked_add(config.warmup)
            .ok_or("batch count overflow")?;
        let operations = total_batches
            .checked_mul(config.iterations)
            .ok_or("operation count overflow")?;
        let bound = (operations as u128 + config.width as u128)
            .checked_mul(config.width.max(config.iterations) as u128)
            .ok_or("checksum bound overflow")?;
        if bound > (1_u128 << 52) {
            return Err("parameters exceed exact Lua checksum arithmetic budget".into());
        }
        if config.workload != "all"
            && !WORKLOADS
                .iter()
                .any(|(name, _, _)| *name == config.workload)
        {
            return Err(format!("unknown workload {}", config.workload));
        }
        Ok(config)
    }
}

fn snapshot(runtime: &mut Runtime) -> Result<Json, String> {
    let (objects, accounted) = {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        let (_, gc, _) = parts.split_mut();
        (gc.object_count(), gc.total_memory())
    };
    let allocator = runtime.allocator_stats();
    Ok(json!({
        "gc_objects": objects,
        "gc_accounted_bytes": accounted,
        "managed_live_bytes": allocator.live_bytes,
        "managed_lifetime_peak_bytes": allocator.peak_bytes,
        "managed_lifetime_allocated_bytes": allocator.total_allocated_bytes,
    }))
}

fn compile_rooted(runtime: &mut Runtime, source: &str, name: &str) -> Result<GcRef<Proto>, String> {
    let chunk = Parser::new(source)
        .parse()
        .map_err(|error| error.to_string())?;
    let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
    let (_, gc, strings) = parts.split_mut();
    let proto = CodeGenerator::new_with_pool(gc, strings)
        .generate(&chunk, name)
        .map_err(|error| error.to_string())?;
    let proto = gc.create(proto);
    gc.add_root(proto);
    Ok(proto)
}

fn execute(runtime: &mut Runtime, proto: GcRef<Proto>, args: [usize; 5]) -> Result<f64, String> {
    let mut checksum = None;
    runtime
        .execute_proto_with_args(
            proto,
            args.into_iter()
                .map(|value| Value::Number(value as f64))
                .collect(),
            |_, values| {
                if let [Value::Number(value)] = values {
                    checksum = Some(*value);
                }
            },
        )
        .map_err(|error| error.to_string())?;
    checksum
        .filter(|value| value.is_finite())
        .ok_or_else(|| "workload must return one finite numeric checksum".into())
}

fn timed_collection(runtime: &mut Runtime) -> Result<(u64, usize), String> {
    let start = Instant::now();
    let collected = runtime.collect_garbage_full()?;
    let elapsed = start.elapsed().as_nanos();
    Ok((
        u64::try_from(elapsed).map_err(|_| "duration exceeds u64 nanoseconds")?,
        collected,
    ))
}

fn distribution(samples: &[u64]) -> Json {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = |percent: usize| sorted[(percent * sorted.len()).div_ceil(100) - 1];
    json!({
        "samples": sorted.len(), "min_ns": sorted[0], "max_ns": sorted[sorted.len() - 1],
        "p50_ns": percentile(50), "p95_ns": percentile(95), "p99_ns": percentile(99),
        "mean_ns": sorted.iter().map(|value| *value as f64).sum::<f64>() / sorted.len() as f64,
    })
}

fn run_workload(config: &Config, name: &str, source: &str, rotating: bool) -> Result<Json, String> {
    let mut runtime = Runtime::try_new().map_err(|error| error.to_string())?;
    runtime
        .parts_mut()
        .map_err(|error| error.to_string())?
        .with_context_gc(lua_stdlib::open_all);
    let compile_start = Instant::now();
    let proto = compile_rooted(&mut runtime, source, &format!("@baseline/{name}"))?;
    let compile_ns = compile_start.elapsed().as_nanos();
    runtime.collect_garbage_full()?;
    let baseline = snapshot(&mut runtime)?;
    execute(
        &mut runtime,
        proto,
        [0, config.iterations, config.width, 0, usize::from(rotating)],
    )?;
    runtime.collect_garbage_full()?;
    let after_setup = snapshot(&mut runtime)?;

    let mut samples = Vec::new();
    let mut execution_times = Vec::new();
    let mut collection_times = Vec::new();
    let mut after_warmup = after_setup.clone();
    for batch in 1..=(config.warmup + config.batches) {
        let before = snapshot(&mut runtime)?;
        let start = Instant::now();
        let checksum = execute(
            &mut runtime,
            proto,
            [
                1,
                config.iterations,
                config.width,
                batch,
                usize::from(rotating),
            ],
        )?;
        let execution_ns = u64::try_from(start.elapsed().as_nanos())
            .map_err(|_| "duration exceeds u64 nanoseconds")?;
        let before_gc = snapshot(&mut runtime)?;
        let (explicit_gc_ns, collected_objects) = timed_collection(&mut runtime)?;
        let after_gc = snapshot(&mut runtime)?;
        if batch <= config.warmup {
            after_warmup = after_gc;
        } else {
            execution_times.push(execution_ns);
            collection_times.push(explicit_gc_ns);
            samples.push(json!({
                "batch": batch - config.warmup, "lifetime_batch": batch,
                "operations": config.iterations, "checksum": checksum,
                "execution_wall_ns": execution_ns, "explicit_full_gc_wall_ns": explicit_gc_ns,
                "explicit_collected_objects": collected_objects,
                "before_execution": before, "before_explicit_gc": before_gc, "after_explicit_gc": after_gc,
            }));
        }
    }
    let retained = snapshot(&mut runtime)?;
    execute(
        &mut runtime,
        proto,
        [2, config.iterations, config.width, 0, usize::from(rotating)],
    )?;
    // Two collections make post-release observations robust to finalizer staging.
    runtime.collect_garbage_full()?;
    runtime.collect_garbage_full()?;
    let after_release = snapshot(&mut runtime)?;
    {
        let mut parts = runtime.parts_mut().map_err(|error| error.to_string())?;
        parts.split_mut().1.remove_root(proto);
    }
    runtime.close().map_err(|error| error.to_string())?;
    Ok(json!({
        "name": name, "runtime_instances": 1, "compile_wall_ns": compile_ns,
        "baseline_with_rooted_proto": baseline, "after_setup": after_setup,
        "after_warmup": after_warmup, "retained_after_measurements": retained,
        "after_release_and_two_full_collections": after_release,
        "execution_wall": distribution(&execution_times),
        "explicit_full_gc_wall": distribution(&collection_times), "batches": samples,
    }))
}

fn run() -> Result<Json, String> {
    let config = Config::parse()?;
    let mut workloads = Vec::new();
    for &(name, source, rotating) in WORKLOADS {
        if config.workload == "all" || config.workload == name {
            workloads.push(
                run_workload(&config, name, source, rotating)
                    .map_err(|error| format!("{name}: {error}"))?,
            );
        }
    }
    Ok(json!({
        "schema_version": 1, "completed": true, "debug_assertions": cfg!(debug_assertions),
        "architecture": env::consts::ARCH, "os": env::consts::OS,
        "parameters": {"iterations": config.iterations, "width": config.width, "batches": config.batches, "warmup": config.warmup},
        "measurement_contract": {
            "clock": "std::time::Instant host wall time; nanoseconds",
            "execution": "execute_proto_with_args including argument/result handling and workload assertions; excludes compilation, snapshots, explicit GC",
            "explicit_gc": "idle-host Runtime::collect_garbage_full method wall time; excludes snapshots; not automatic GC pause instrumentation",
            "automatic_gc": "production defaults unchanged; any automatic GC work is included in execution wall time",
            "managed_bytes": "allocator ledger payload/layout/container capacity and Runtime state accounting; not OS RSS or allocator usable size",
            "managed_peak": "cumulative Runtime lifetime peak, including setup, compilation, and warmup; not a resettable per-batch peak",
            "gc_accounted_bytes": "collector estimated object accounting, narrower than the managed allocator ledger",
            "percentiles": "nearest rank: sorted[ceil(p * N / 100) - 1]; warmup and setup GC excluded; small N cannot characterize tails reliably",
            "runtime_scope": "fresh Runtime per workload; same Runtime and retained Lua objects across all warmup and measured batches",
        },
        "workloads": workloads,
    }))
}

fn main() {
    match run() {
        Ok(report) => println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("JSON report serialization")
        ),
        Err(error) => {
            eprintln!("runtime baseline failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::distribution;

    #[test]
    fn percentile_rank_preserves_observed_tail_samples() {
        let values: Vec<u64> = (1..=100).rev().collect();
        let summary = distribution(&values);
        assert_eq!(summary["p50_ns"], 50);
        assert_eq!(summary["p95_ns"], 95);
        assert_eq!(summary["p99_ns"], 99);
        let small = distribution(&[40, 10, 30, 20]);
        assert_eq!(small["p50_ns"], 20);
        assert_eq!(small["p95_ns"], 40);
        assert_eq!(small["p99_ns"], 40);
    }
}
