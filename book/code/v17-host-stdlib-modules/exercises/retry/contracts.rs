use crate::model::{self, Attempt, Loader, RetryVm};
use lua_core::Value;

pub fn failed_entry_is_retryable(attempt: Attempt) {
    for initial_false in [false, true] {
        for loader in [model::fail as Loader, model::publish_then_fail as Loader] {
            let mut vm = RetryVm::new(attempt);
            if initial_false {
                vm.loaded.insert("broken".to_owned(), Value::Boolean(false));
            }
            assert_eq!(
                vm.require("broken", loader),
                Err(model::loader_error("broken")),
                "清理后必须保留原错误种类/位置/文本"
            );
            assert_eq!(
                vm.cached("broken"),
                Value::Nil,
                "失败只删当前 key，不恢复 old false 或留下发布值"
            );
            assert_eq!(vm.cached("outer"), Value::Number(900.0));
            assert_eq!(vm.side_effects, 1, "loader 的副作用不回滚");
            assert_eq!(
                vm.require("broken", model::repaired),
                Ok(Value::Number(42.0))
            );
            assert_eq!(
                vm.require("broken", model::repaired),
                Ok(Value::Number(42.0))
            );
            assert_eq!(vm.calls["broken"], 2, "修复后确实再执行一次，随后命中缓存");
            assert_eq!(vm.cached("outer"), Value::Number(900.0));
        }
    }
    let mut vm = RetryVm::new(attempt);
    assert_eq!(
        vm.require("parent", model::depends_then_fails),
        Err(model::loader_error("parent"))
    );
    assert_eq!(vm.cached("parent"), Value::Nil);
    assert_eq!(
        vm.cached("dependency"),
        Value::Number(41.0),
        "成功依赖属于独立缓存，不能 clear_all"
    );
    assert_eq!(vm.cached("outer"), Value::Number(900.0));
    assert_eq!(
        vm.require("parent", model::depends_then_succeeds),
        Ok(Value::Number(42.0))
    );
    assert_eq!(vm.calls["parent"], 2);
    assert_eq!(vm.calls["dependency"], 1);
    assert_eq!(vm.side_effects, 3);
}
