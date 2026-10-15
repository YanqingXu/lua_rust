//! 正向真实 VM 探针不执行已知 snapshot roots 失败输入；后者必须单独运行与统计。
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;

// ANCHOR: sort_roots_real_normal
pub fn vm_normal_sort_and_truthiness() -> Value {
    let source = br#"
      local function make_source()
        return { { rank = 4 }, { rank = 3 }, { rank = 2 }, { rank = 1 } }
      end
      local source = make_source()
      table.sort(source, function(left, right)
        gc_collect()
        return left.rank < right.rank
      end)
      local a, b, c = { 2, 1 }, { 2, 1 }, { 2, 1 }
      table.sort(a, function() return false end)
      table.sort(b, function() return nil end)
      table.sort(c, function() return 0 end)
      return source[1].rank == 1 and source[4].rank == 4
        and a[1] == 2 and a[2] == 1 and b[1] == 2 and b[2] == 1
        and c[1] == 1 and c[2] == 2
    "#;
    Vm::new().execute(&compile(source).unwrap()).unwrap()
}
// ANCHOR_END: sort_roots_real_normal

// ANCHOR: sort_roots_real_error
pub fn vm_comparator_error_keeps_its_source_mutation() -> Value {
    let source = br#"
      local source = { 4, 3, 2, 1 }
      local calls = 0
      local ok, message = pcall(table.sort, source, function(left, right)
        calls = calls + 1
        if calls == 2 then
          source[1] = 99
          error('sort-fail')
        end
        return left < right
      end)
      return not ok and calls == 2 and source[1] == 99 and source[2] == 3
        and type(message) == 'string' and string.find(message, 'sort-fail', 1, true) ~= nil
    "#;
    Vm::new().execute(&compile(source).unwrap()).unwrap()
}
// ANCHOR_END: sort_roots_real_error
