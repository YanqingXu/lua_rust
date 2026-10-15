use crate::contracts::equal;
use crate::model::{exclusive, Exclusive};
use crate::temp::{lua_quote, TempFiles};
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;

fn run(vm: &mut Vm, source: &[u8]) -> Result<Value, String> {
    let proto = compile(source).map_err(|error| error.to_string())?;
    vm.execute(&proto).map_err(|error| error.to_string())
}

// ANCHOR: file_prune_real_vm
const KEPT_AND_CLEARED: &[u8] = br#"
local closed_ok, closed_error = pcall(function() return kept_closed:read(1) end)
assert(kept_open:write('still usable') == kept_open)
assert(kept_open:flush())
return weak.dropped_open == nil and weak.dropped_closed == nil
  and weak.kept_open == kept_open and weak.kept_closed == kept_closed
  and io.type(kept_open) == 'file' and io.type(kept_closed) == 'closed file'
  and not closed_ok and string.find(closed_error, 'closed file') ~= nil
"#;
// ANCHOR_END: file_prune_real_vm

#[derive(Debug, PartialEq)]
pub struct VmTrace {
    pub checks: [Value; 3],
    pub before: [Exclusive; 4],
    pub after: [Exclusive; 4],
    pub released: [Exclusive; 4],
}

pub fn vm_weak_slots_and_os_release() -> Result<VmTrace, String> {
    let directory = TempFiles::new()?;
    let result = (|| {
        let paths = std::array::from_fn::<_, 4, _>(|index| directory.file(index));
        let source = format!(
            r#"
weak = setmetatable({{}}, {{ __mode = 'v' }})
kept_open = assert(io.open({}, 'w+'))
kept_closed = assert(io.open({}, 'w+'))
assert(kept_closed:close())
do
  local dropped_open = assert(io.open({}, 'w+'))
  local dropped_closed = assert(io.open({}, 'w+'))
  assert(dropped_closed:close())
  weak.dropped_open, weak.dropped_closed = dropped_open, dropped_closed
end
weak.kept_open, weak.kept_closed = kept_open, kept_closed
return true
"#,
            lua_quote(&paths[0])?,
            lua_quote(&paths[1])?,
            lua_quote(&paths[2])?,
            lua_quote(&paths[3])?
        );
        let mut vm = Vm::new();
        let first = run(&mut vm, source.as_bytes())?;
        equal(
            "created four real file userdata",
            first.clone(),
            Value::Boolean(true),
        )?;
        let probe = || -> Result<[Exclusive; 4], String> {
            Ok([
                exclusive(&paths[0])?,
                exclusive(&paths[1])?,
                exclusive(&paths[2])?,
                exclusive(&paths[3])?,
            ])
        };
        let before = probe()?;
        // execute ended: no construction frame remains as an unintended root.
        vm.collect_garbage().map_err(|error| error.to_string())?;
        let after = probe()?;
        let second = run(&mut vm, KEPT_AND_CLEARED)?;
        equal(
            "weak nil plus actual open/closed operations",
            second.clone(),
            Value::Boolean(true),
        )?;
        equal(
            "release global owners",
            run(&mut vm, b"kept_open, kept_closed = nil, nil; return true")?,
            Value::Boolean(true),
        )?;
        vm.collect_garbage().map_err(|error| error.to_string())?;
        let released = probe()?;
        let third = run(&mut vm, b"return weak.kept_open == nil and weak.kept_closed == nil and weak.dropped_open == nil and weak.dropped_closed == nil")?;
        equal(
            "all weak slots exactly nil",
            third.clone(),
            Value::Boolean(true),
        )?;
        if cfg!(windows) {
            use Exclusive::{Available as A, Blocked as B};
            equal("OS before", before, [B, A, B, A])?;
            equal("OS after unreachable owners pruned", after, [B, A, A, A])?;
            equal("OS after all owners released", released, [A; 4])?;
        } else {
            equal(
                "no equivalent non-Windows OS evidence",
                before,
                [Exclusive::Unsupported; 4],
            )?;
        }
        Ok(VmTrace {
            checks: [first, second, third],
            before,
            after,
            released,
        })
        // Vm drops before directory cleanup, including on a returned error.
    })();
    directory.finish(result)
}
