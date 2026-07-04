//! GC 终结器处理实现
//!
//! 管理 userdata 的 `__gc` 终结器：在标记阶段复活带终结器的不可达
//! userdata，将其加入待终结队列，并在 sweep 后执行终结器。
//!
//! Phase 1.3 状态：Userdata 类型尚未实现，本模块提供方法框架。
//! 完整的终结器逻辑将在 Phase 3（VM）且 Userdata 实现后启用。
//!
//! C++ 参考: `lua_cpp/src/gc/gc_finalize.cpp`

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_ref::GcRef;
use crate::gc_string::GcString;
use crate::table::Table;
use crate::types::GcObjectType;
use crate::userdata::Userdata;
use crate::value::Value;

impl GarbageCollector {
    /// 准备终结器：将带 `__gc` 的不可达 userdata 复活并加入待终结队列
    ///
    /// 在标记阶段之后、sweep 之前调用。
    /// 被复活的 userdata 及其引用图将在下一轮 propagate 中标记。
    ///
    /// Phase 1.3: 骨架实现 — Userdata 类型尚未实现，当前为空操作。
    ///
    /// C++ 对应: `GarbageCollector::prepareFinalizers()`
    pub fn prepare_finalizers(&mut self) {
        // Phase 1.4+: 遍历 allObjects_，查找白色、非固定、非已终结的 Userdata
        // 检查其 metatable 中是否有 __gc 元方法
        // 如果有：标记 FINALIZED，加入 pendingFinalizers_，调用 markObject 复活
        //
        // 当前骨架：无操作（Userdata 未实现）
    }

    /// Return unreachable userdata that have a `__gc` metamethod.
    ///
    /// This is a VM-facing compatibility hook used before weak-table cleanup.
    /// It marks selected userdata as finalized and keeps their pointers in
    /// `pending_finalizers` so weak values can be cleared before `__gc` runs.
    pub fn prepare_finalizable_userdata(&mut self) -> Vec<GcRef<Userdata>> {
        let mut pending = Vec::new();
        let mut current = self.all_objects;

        while !current.is_null() {
            // SAFETY: current walks the GC intrusive object list.
            let next = unsafe { (*current).next() };
            let should_finalize = unsafe {
                (*current).gc_type() == GcObjectType::Userdata
                    && (*current).is_white()
                    && !(*current).is_finalized()
                    && userdata_has_gc(current as *const Userdata)
            };

            if should_finalize {
                // SAFETY: current is a valid userdata object from the GC list.
                unsafe {
                    (*current).mark_finalized();
                    self.mark_object(current);
                    pending.push(GcRef::from_ptr(current as *const Userdata));
                }
                if !self.pending_finalizers.contains(&current) {
                    self.pending_finalizers.push(current);
                }
            }

            current = next;
        }

        pending
    }

    pub fn clear_pending_finalizers(&mut self) {
        self.pending_finalizers.clear();
    }

    /// 运行待终结队列中的终结器
    ///
    /// 在 sweep 完成后调用。逐个执行 pendingFinalizers_ 中 userdata
    /// 的 `__gc` 元方法。防止终结器递归执行。
    ///
    /// Phase 1.3: 骨架实现 — 需要 LuaState 和 VM 支持。
    ///
    /// C++ 对应: `GarbageCollector::runFinalizers(LuaState* state)`
    #[allow(unused_variables)]
    pub fn run_finalizers(&mut self) {
        // Phase 3+: 需要 LuaState 来调用 __gc 函数
        // 将 pendingFinalizers_ swap 到局部列表
        // 对每个 userdata 调用 __gc 元方法
        // 处理异常（保存后续终结器）
        //
        // 当前骨架：无操作
    }
}

unsafe fn userdata_has_gc(userdata_ptr: *const Userdata) -> bool {
    // SAFETY: caller provides a valid userdata pointer from the GC object list.
    let Some(metatable) = (unsafe { (*userdata_ptr).metatable() }) else {
        return false;
    };
    metatable_has_field(metatable, "__gc")
}

fn metatable_has_field(metatable: GcRef<Table>, name: &str) -> bool {
    let Some(table) = (unsafe { metatable.as_ref() }) else {
        return false;
    };
    table.hash_entries().any(|(key, value)| {
        !value.is_nil()
            && matches!(
                key,
                Value::String(key_ref)
                    if unsafe { key_ref.as_ref() }
                        .is_some_and(|key_string: &GcString| key_string.data() == name)
            )
    })
}
