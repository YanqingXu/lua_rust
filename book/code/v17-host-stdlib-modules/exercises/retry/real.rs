//! 真实 VM/file 探针：不改源码，不更改进程 cwd，不用新 VM 假装失败后重试。
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn run(vm: &mut Vm, source: &[u8]) -> Value {
    vm.execute(&compile(source).unwrap()).unwrap()
}

// ANCHOR: retry_real_preload
pub const PRELOAD_RETRY: &[u8] = br#"
local calls = 0
local effects = 0
package.loaded.retry_preload = false
package.preload.retry_preload = function(name)
  calls = calls + 1
  effects = effects + 1
  package.loaded[name] = 'published before failure'
  error('first load failed')
end
local ok, message = pcall(require, 'retry_preload')
local cleared = package.loaded.retry_preload == nil
package.preload.retry_preload = function()
  calls = calls + 1
  return { answer = 42 }
end
local result = require('retry_preload')
local again = require('retry_preload')
return not ok and type(message) == 'string' and cleared and effects == 1
  and calls == 2 and result.answer == 42 and result == again
"#;
// ANCHOR_END: retry_real_preload

pub fn preload_retry() -> Value {
    run(&mut Vm::new(), PRELOAD_RETRY)
}

pub fn nested_dependency_survives_parent_failure() -> Value {
    run(
        &mut Vm::new(),
        br#"
        local a_runs, b_runs, effects = 0,0,0
        package.loaded.retry_outer = 900
        package.preload.retry_dependency = function()
          b_runs = b_runs + 1
          return { answer = 41 }
        end
        package.preload.retry_parent = function(name)
          a_runs = a_runs + 1
          require('retry_dependency')
          package.loaded[name] = 'partial'
          effects = effects + 1
          error('parent failed')
        end
        local ok = pcall(require,'retry_parent')
        local cleared = package.loaded.retry_parent == nil
        local dependency = package.loaded.retry_dependency
        package.preload.retry_parent = function()
          a_runs = a_runs + 1
          return require('retry_dependency').answer + 1
        end
        local result = require('retry_parent')
        return not ok and cleared and a_runs == 2 and b_runs == 1 and effects == 1
          and dependency.answer == 41 and dependency == package.loaded.retry_dependency
          and package.loaded.retry_outer == 900 and result == 42
    "#,
    )
}

// ANCHOR: retry_real_yield
pub fn rejected_yield_then_retry_and_yield() -> Value {
    run(
        &mut Vm::new(),
        br#"
      local co = coroutine.create(function()
        package.preload.retry_yield = function() return coroutine.yield(99) end
        package.loaded.retry_yield = false
        local ok, message = pcall(require,'retry_yield')
        local clean = not ok and type(message)=='string'
          and string.find(message,'yield',1,true)~=nil and package.loaded.retry_yield==nil
        package.preload.retry_yield = function() return { answer=42 } end
        local module = require('retry_yield')
        local resumed = coroutine.yield(clean and module.answer==42)
        return resumed + 1
      end)
      local ok1, result = coroutine.resume(co)
      local ok2, answer = coroutine.resume(co,42)
      return ok1 and result==true and ok2 and answer==43 and coroutine.status(co)=='dead'
    "#,
    )
}
// ANCHOR_END: retry_real_yield

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Files {
    root: PathBuf,
    cleanup_parent: PathBuf,
    active: bool,
}

impl Files {
    fn new() -> Self {
        let temp_root = std::env::temp_dir();
        assert!(
            temp_root.is_absolute(),
            "fixture temporary root must be absolute"
        );
        let cleanup_parent = fs::canonicalize(&temp_root).unwrap();
        let root = temp_root.join(format!(
            "lua_rust_v17_retry_{}_{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        // create_dir 拒绝已有目录，不能接管另一个调用的文件。
        fs::create_dir(&root).unwrap();
        Self {
            root,
            cleanup_parent,
            active: true,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn write(&self, name: &str, source: &[u8]) {
        fs::write(self.path(name), source).unwrap();
    }
    fn template(&self) -> String {
        path_template(&self.root)
    }

    fn remove_checked(&self) -> std::io::Result<()> {
        let resolved = fs::canonicalize(&self.root)?;
        if resolved.parent() != Some(self.cleanup_parent.as_path())
            || resolved.file_name() != self.root.file_name()
            || !resolved
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("lua_rust_v17_retry_"))
        {
            return Err(std::io::Error::other(
                "cleanup target escaped the task's unique directory",
            ));
        }
        fs::remove_dir_all(resolved)
    }

    fn finish<T>(mut self, result: T) -> T {
        self.active = false;
        self.remove_checked()
            .expect("normal fixture cleanup must succeed");
        result
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.remove_checked() {
                eprintln!("retry fixture cleanup during early exit: {error}");
            }
        }
    }
}

fn lua_quote(text: &str) -> String {
    let escaped = text
        .bytes()
        .map(|byte| format!("\\{byte:03}"))
        .collect::<String>();
    format!("\"{escaped}\"")
}

fn run_text(vm: &mut Vm, source: String) -> Value {
    run(vm, source.as_bytes())
}

pub fn missing_file_can_be_created_and_retried() -> [Value; 2] {
    let files = Files::new();
    let mut vm = Vm::new();
    let before = run_text(
        &mut vm,
        format!(
            r#"
      package.path={}
      package.loaded.retry_missing=false
      local ok,message=pcall(require,'retry_missing')
      return not ok and type(message)=='string' and package.loaded.retry_missing==false
    "#,
            lua_quote(&files.template())
        ),
    );
    files.write("retry_missing.lua", b"return { answer=42 }");
    let after = run(&mut vm, b"local value=require('retry_missing'); return value.answer==42 and value==package.loaded.retry_missing");
    files.finish([before, after])
}

pub fn compile_error_stops_before_a_later_template_then_retries() -> [Value; 2] {
    let files = Files::new();
    let first = files.path("first");
    let second = files.path("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(first.join("retry_compile.lua"), b"local = invalid").unwrap();
    fs::write(
        second.join("retry_compile.lua"),
        b"retry_fallback_runs=(retry_fallback_runs or 0)+1; return 99",
    )
    .unwrap();
    let path = format!("{};{}", path_template(&first), path_template(&second));
    let mut vm = Vm::new();
    let before = run_text(
        &mut vm,
        format!(
            r#"
      package.path={}
      package.loaded.retry_compile=false
      local ok,message=pcall(require,'retry_compile')
      return not ok and type(message)=='string' and package.loaded.retry_compile==false
        and retry_fallback_runs==nil
    "#,
            lua_quote(&path)
        ),
    );
    fs::write(
        first.join("retry_compile.lua"),
        b"retry_compile_runs=(retry_compile_runs or 0)+1; return { answer=42 }",
    )
    .unwrap();
    let after = run(&mut vm, b"local value=require('retry_compile'); return value.answer==42 and retry_compile_runs==1 and retry_fallback_runs==nil");
    files.finish([before, after])
}

fn path_template(directory: &Path) -> String {
    directory
        .join("?.lua")
        .to_str()
        .expect("fixture path must be UTF-8")
        .replace('\\', "/")
}

pub fn runtime_failure_clears_marker_then_same_vm_retries() -> [Value; 2] {
    let files = Files::new();
    files.write(
        "retry_runtime.lua",
        b"retry_runtime_runs=(retry_runtime_runs or 0)+1; error('runtime failed')",
    );
    let mut vm = Vm::new();
    let before = run_text(
        &mut vm,
        format!(
            r#"
      package.path={}
      package.loaded.retry_runtime=false
      local ok,message=pcall(require,'retry_runtime')
      return not ok and type(message)=='string' and package.loaded.retry_runtime==nil
        and retry_runtime_runs==1
    "#,
            lua_quote(&files.template())
        ),
    );
    files.write(
        "retry_runtime.lua",
        b"retry_runtime_runs=retry_runtime_runs+1; return { answer=42 }",
    );
    let after = run(
        &mut vm,
        br#"
      local value=require('retry_runtime')
      gc_collect()
      local again=require('retry_runtime')
      return value.answer==42 and value==again and retry_runtime_runs==2
    "#,
    );
    files.finish([before, after])
}
