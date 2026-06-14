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
