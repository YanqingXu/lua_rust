//! GC 策略 trait 和内置实现
//!
//!

use crate::gc::collector::GarbageCollector;
use crate::string_pool::StringPool;

/// GC 上下文 — 收集循环所需的参数
///
pub struct GcContext<'a> {
    /// 垃圾回收器
    pub collector: &'a mut GarbageCollector,
    /// 字符串驻留池
    pub string_pool: &'a mut StringPool,
}

/// GC 策略 trait
///
pub trait GcStrategy {
    /// 执行收集循环，返回回收的对象数量
    fn collect(&self, context: &mut GcContext<'_>) -> usize;

    /// 策略名称
    fn name(&self) -> &'static str;

    /// 策略摘要
    fn summary(&self) -> &'static str;
}

// =====================================================================
// 标记-清除策略（默认）
// =====================================================================

/// 标准 stop-the-world 标记-清除 GC 策略
///
pub struct MarkSweepGc;

impl GcStrategy for MarkSweepGc {
    fn collect(&self, context: &mut GcContext<'_>) -> usize {
        // Phase 1.3: 完整实现标记 → 传播 → 弱表清理 → 终结 → 清除
        context.collector.collect(context.string_pool)
    }

    fn name(&self) -> &'static str {
        "mark-sweep"
    }

    fn summary(&self) -> &'static str {
        "Standard stop-the-world mark-and-sweep garbage collector"
    }
}

// =====================================================================
// 增量 GC 策略（占位）
// =====================================================================

/// 增量垃圾回收策略（教学占位）
///
/// 当前行为等价于 MarkSweepGC，预留增量接口。
///
pub struct IncrementalGc;

impl GcStrategy for IncrementalGc {
    fn collect(&self, context: &mut GcContext<'_>) -> usize {
        // Phase 1.3+: 增量步进实现
        context.collector.collect(context.string_pool)
    }

    fn name(&self) -> &'static str {
        "incremental"
    }

    fn summary(&self) -> &'static str {
        "Incremental garbage collector (currently equivalent to mark-sweep)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_names() {
        let ms = MarkSweepGc;
        let inc = IncrementalGc;

        assert_eq!(ms.name(), "mark-sweep");
        assert!(!ms.summary().is_empty());

        assert_eq!(inc.name(), "incremental");
        assert!(!inc.summary().is_empty());
    }
}
