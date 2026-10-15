use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(vm: &mut Vm, source: &[u8]) -> Result<Value, String> {
    let proto = compile(source).map_err(|error| error.to_string())?;
    vm.execute(&proto).map_err(|error| error.to_string())
}

// ANCHOR: cycle_real_preload
pub const PRELOAD_CYCLES: &[u8] = br#"
self_runs, a_runs, b_runs = 0, 0, 0
package.preload.cycle_self = function(name)
  self_runs = self_runs + 1
  return { name = name, inner = require(name), answer = 19 }
end
package.preload.cycle_a = function()
  a_runs = a_runs + 1
  return { kind = 'A', peer = require('cycle_b'), answer = 42 }
end
package.preload.cycle_b = function()
  b_runs = b_runs + 1
  return { kind = 'B', peer = require('cycle_a'), answer = 7 }
end
local single = require('cycle_self')
local a, b = require('cycle_a'), require('cycle_b')
return single.inner == true and single.name == 'cycle_self' and single.answer == 19
  and single == require('cycle_self') and a.kind == 'A' and a.answer == 42
  and b.kind == 'B' and b.answer == 7 and a.peer == b and b.peer == true
  and package.loaded.cycle_a == a and package.loaded.cycle_b == b
  and self_runs == 1 and a_runs == 1 and b_runs == 1
"#;
// ANCHOR_END: cycle_real_preload

pub fn vm_preload_cycles() -> Result<Value, String> {
    run(&mut Vm::new(), PRELOAD_CYCLES)
}

pub fn vm_explicit_partial_and_completed_true() -> Result<Value, String> {
    run(
        &mut Vm::new(),
        br#"
      local partial_runs, true_runs = 0, 0
      package.preload.partial = function(name)
        partial_runs = partial_runs + 1
        local exports = { ready = false }
        package.loaded[name] = exports
        local again = require(name)
        exports.saw_unready = again == exports and again.ready == false
        exports.ready = true
        return exports
      end
      package.preload.done_true = function()
        true_runs = true_runs + 1
        return true
      end
      local partial = require('partial')
      local first, second = require('done_true'), require('done_true')
      return partial.saw_unready and partial.ready and partial_runs == 1
        and partial == package.loaded.partial and first == true and second == true
        and package.loaded.done_true == true and true_runs == 1
    "#,
    )
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempModules {
    path: Option<PathBuf>,
    root: PathBuf,
}
impl TempModules {
    fn new() -> Result<Self, String> {
        let raw_root = std::env::temp_dir();
        if !raw_root.is_absolute() {
            return Err("temporary root must be absolute".to_owned());
        }
        let root = fs::canonicalize(&raw_root).map_err(|error| error.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path = raw_root.join(format!(
            "lua-rust-v17-cycle-{}-{nonce}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            path: Some(path),
            root,
        })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().unwrap()
    }

    fn remove_checked(&self, path: &Path) -> Result<(), String> {
        let resolved =
            fs::canonicalize(path).map_err(|error| format!("resolve cleanup target: {error}"))?;
        if resolved.parent() != Some(self.root.as_path())
            || !resolved
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("lua-rust-v17-cycle-"))
        {
            return Err("cleanup target escaped the task's unique temporary directory".to_owned());
        }
        fs::remove_dir_all(&resolved)
            .map_err(|error| format!("cleanup {}: {error}", resolved.display()))
    }

    fn finish<T>(mut self, result: Result<T, String>) -> Result<T, String> {
        let path = self.path.take().unwrap();
        let cleanup = self.remove_checked(&path);
        match (result, cleanup) {
            (Err(primary), Err(cleanup)) => Err(format!("{primary}; additionally {cleanup}")),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

impl Drop for TempModules {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            if let Err(error) = self.remove_checked(path) {
                eprintln!("temporary module cleanup during early exit: {error}");
            }
        }
    }
}

fn lua_quote(text: &str) -> String {
    let bytes = text
        .bytes()
        .map(|byte| format!("\\{byte:03}"))
        .collect::<String>();
    format!("\"{bytes}\"")
}

pub fn vm_file_cycles() -> Result<[Value; 2], String> {
    let modules = TempModules::new()?;
    let result = (|| {
        for (name, source) in [
            ("cycle_self.lua", "file_self_runs = (file_self_runs or 0) + 1; return { inner = require('cycle_self'), answer = 19 }"),
            ("cycle_a.lua", "file_a_runs = (file_a_runs or 0) + 1; return { kind = 'A', peer = require('cycle_b'), answer = 42 }"),
            ("cycle_b.lua", "file_b_runs = (file_b_runs or 0) + 1; return { kind = 'B', peer = require('cycle_a'), answer = 7 }"),
        ] { fs::write(modules.path().join(name), source).map_err(|error| error.to_string())?; }
        let path = modules.path().join("?.lua");
        let path = path
            .to_str()
            .ok_or_else(|| "module path must be UTF-8".to_owned())?;
        let source = format!(
            r#"
          package.path = {}
          local single = require('cycle_self')
          local a, b = require('cycle_a'), require('cycle_b')
          return single.inner == true and single.answer == 19 and single == package.loaded.cycle_self
            and a.peer == b and b.peer == true and a.kind == 'A' and a.answer == 42
            and b.kind == 'B' and b.answer == 7 and a == require('cycle_a')
            and b == require('cycle_b') and file_self_runs == 1 and file_a_runs == 1 and file_b_runs == 1
        "#,
            lua_quote(path)
        );
        let mut vm = Vm::new();
        let first = run(&mut vm, source.as_bytes())?;
        vm.collect_garbage().map_err(|error| error.to_string())?;
        let kept = run(
            &mut vm,
            br#"
          return package.loaded.cycle_a.peer == package.loaded.cycle_b
            and package.loaded.cycle_a.answer == 42 and package.loaded.cycle_b.answer == 7
            and package.loaded.cycle_b.peer == true and file_a_runs == 1 and file_b_runs == 1
        "#,
        )?;
        Ok([first, kept])
    })();
    modules.finish(result)
}
