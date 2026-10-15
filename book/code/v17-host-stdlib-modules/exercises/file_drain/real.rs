//! 生产 Heap/VM 对照：不注入模型 Hook，不读取 VM 私有 files。
use crate::support::{lua_path, TempDirectory};
use lua_compiler::compile;
use lua_core::{Heap, HeapError, NativeFunction, Value};
use lua_vm::Vm;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    pub reachable_was_unfinalized: bool,
    pub pending_was_finalized: bool,
    pub released: usize,
    pub remaining: usize,
    pub access_rejected: [bool; 2],
    pub events_after_shutdown: usize,
    pub allocation_rejected: bool,
}

// ANCHOR: file_drain_heap_shutdown
pub fn heap_shutdown_discards_pending_events() -> Result<ShutdownReport, String> {
    let mut heap = Heap::new();
    let reachable = heap
        .allocate_userdata()
        .map_err(|error| error.to_string())?;
    let pending = heap
        .allocate_userdata()
        .map_err(|error| error.to_string())?;
    for handle in [reachable, pending] {
        heap.userdata_set_finalizer(
            handle,
            Some(Value::NativeFunction(NativeFunction::ToString)),
        )
        .map_err(|error| error.to_string())?;
    }
    heap.collect(&[Value::Userdata(reachable)], &[])
        .map_err(|error| error.to_string())?;
    let reachable_was_unfinalized = heap.userdata_is_finalized(reachable) == Ok(false);
    let pending_was_finalized = heap.userdata_is_finalized(pending) == Ok(true);
    // 不 take 事件：pending 的首死亡周期已经将其排入 ready_finalizers。
    let released = heap.shutdown();
    let access_rejected = [reachable, pending]
        .map(|handle| heap.userdata_metatable(handle) == Err(HeapError::Shutdown));
    Ok(ShutdownReport {
        reachable_was_unfinalized,
        pending_was_finalized,
        released,
        remaining: heap.live_count(),
        access_rejected,
        events_after_shutdown: heap.take_finalizers().len(),
        allocation_rejected: heap.allocate_userdata() == Err(HeapError::Shutdown),
    })
}
// ANCHOR_END: file_drain_heap_shutdown

#[derive(Debug, PartialEq, Eq)]
pub struct ReleaseReport {
    pub same_vm_global_open: bool,
    pub windows_exclusive_before_blocked: Option<bool>,
    pub windows_exclusive_after_opened: Option<bool>,
    pub contents: Vec<u8>,
}

fn execute_true(vm: &mut Vm, source: &str) -> Result<(), String> {
    let proto = compile(source.as_bytes()).map_err(|error| error.to_string())?;
    let result = vm.execute(&proto).map_err(|error| error.to_string())?;
    if result == Value::Boolean(true) {
        Ok(())
    } else {
        Err(format!("VM probe returned {result:?}, expected true"))
    }
}

#[cfg(windows)]
fn exclusive_before(path: &Path) -> Result<Option<bool>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    match OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
    {
        Err(error) if error.raw_os_error() == Some(32) => Ok(Some(true)),
        Err(error) => Err(format!("expected sharing violation 32, got {error}")),
        Ok(_) => Err("exclusive reopen succeeded while VM still owns the File".into()),
    }
}

#[cfg(not(windows))]
fn exclusive_before(_path: &Path) -> Result<Option<bool>, String> {
    // POSIX 允许同时 open；不把普通 reopen 冒充所有权释放证据。
    Ok(None)
}

fn reopen_after(path: &Path) -> Result<(Option<bool>, Vec<u8>), String> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|error| error.to_string())?;
    Ok((cfg!(windows).then_some(true), contents))
}

// ANCHOR: file_drain_real_vm
pub fn vm_drop_releases_global_file() -> Result<ReleaseReport, String> {
    let directory = TempDirectory::new()?;
    let result = (|| {
        let path = directory.path("vm-owned.bin");
        let quoted = lua_path(&path)?;
        let before;
        {
            let mut vm = Vm::new();
            execute_true(
                &mut vm,
                &format!(
                    "shutdown_file=assert(io.open('{quoted}','w+'))\n\
                     shutdown_file:write('owned');shutdown_file:flush()\n\
                     return io.type(shutdown_file)=='file'"
                ),
            )?;
            vm.collect_garbage().map_err(|error| error.to_string())?;
            // 同一 VM 再 execute，前一脚本只返回 bool；依赖真实 global root。
            execute_true(
                &mut vm,
                "return io.type(shutdown_file)=='file' and \
                 shutdown_file:seek('set',0)==0 and shutdown_file:read('*a')=='owned'",
            )?;
            before = exclusive_before(&path)?;
        } // 生产 Vm::drop：files.clear()；heap.shutdown()。
        let (after, contents) = reopen_after(&path)?;
        Ok(ReleaseReport {
            same_vm_global_open: true,
            windows_exclusive_before_blocked: before,
            windows_exclusive_after_opened: after,
            contents,
        })
    })();
    directory.finish(result)
}
// ANCHOR_END: file_drain_real_vm
