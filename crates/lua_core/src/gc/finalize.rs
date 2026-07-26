//! GC 终结器处理实现
//!
//! 管理 userdata 的 `__gc` 终结器：在标记阶段复活带终结器的不可达
//! userdata，将其加入待终结队列，并在 sweep 后执行终结器。
//!
//! Phase 1.3 状态：Userdata 类型尚未实现，本模块提供方法框架。
//! 完整的终结器逻辑将在 Phase 3（VM）且 Userdata 实现后启用。
//!

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
            let Some(live) = self.live_allocations.get(&(current as usize)).copied() else {
                self.rejected_mark_edges = self.rejected_mark_edges.saturating_add(1);
                break;
            };
            // SAFETY: address membership was established in the authoritative
            // table before reading this internal intrusive-list link.
            let next = unsafe { (*current).next() };
            // SAFETY: address membership and the side-table Userdata tag are
            // checked before reading header mark bits.
            let is_candidate = unsafe {
                live.object_type == GcObjectType::Userdata
                    && (*current).is_white()
                    && !(*current).is_finalized()
            };
            let userdata = is_candidate
                .then(|| self.registered_ref_from_ptr(current.cast::<Userdata>()))
                .flatten();
            let should_finalize = userdata.is_some_and(|userdata| userdata_has_gc(self, userdata));

            if should_finalize {
                // SAFETY: current is a valid userdata object from the GC list.
                unsafe {
                    (*current).mark_finalized();
                }
                self.mark_live_object(current);
                let userdata = userdata.expect("validated finalizer candidate");
                pending.push(userdata);
                if !self.pending_finalizers.contains(&userdata) {
                    self.pending_finalizers.push(userdata);
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

fn userdata_has_gc(gc: &GarbageCollector, userdata: GcRef<Userdata>) -> bool {
    let Ok(metatable) = gc.with_ref(userdata, Userdata::metatable) else {
        return false;
    };
    let Some(metatable) = metatable else {
        return false;
    };
    metatable_has_field(gc, metatable, "__gc")
}

fn metatable_has_field(gc: &GarbageCollector, metatable: GcRef<Table>, name: &str) -> bool {
    let Ok(table) = gc.validate_ref(metatable) else {
        return false;
    };
    // SAFETY: validation matched collector, identity, and Table tag.
    let table = unsafe { table.as_ref() };
    table.hash_entries().any(|(key, value)| {
        !value.is_nil()
            && matches!(
                key,
                Value::String(key_ref) if {
                    gc.with_ref(*key_ref, |key_string: &GcString| {
                        key_string.as_bytes() == name.as_bytes()
                    })
                    .unwrap_or(false)
                }
            )
    })
}
