use crate::model::{key, CycleVm, RunLoader};
use lua_core::Value;

pub fn sentinel_precedes_recursive_loader(run: RunLoader) {
    let mut one = CycleVm::new(run, 6);
    let exports = one
        .require("self")
        .expect("sentinel 必须先于自循环 loader 发布，不能耗尽预算");
    assert_eq!(one.calls.get("self"), Some(&1));
    assert_eq!(one.field(&exports, "inner"), Ok(Value::Boolean(true)));
    assert_eq!(one.field(&exports, "name"), Ok(key("self")));
    assert_eq!(one.field(&exports, "answer"), Ok(Value::Number(42.0)));
    assert_eq!(one.require("self"), Ok(exports.clone()));
    assert_eq!(one.heap.table_get(one.loaded, &key("self")), Ok(exports));
    assert_eq!(
        one.trace,
        [
            "enter self",
            "publish self: boolean",
            "cache self: boolean",
            "publish self: table",
            "cache self: table"
        ]
    );

    let mut pair = CycleVm::new(run, 6);
    let a = pair.require("A").expect("A→B→A 应在内层命中临时 true");
    let b = pair.require("B").unwrap();
    assert_eq!(pair.calls.get("A"), Some(&1));
    assert_eq!(pair.calls.get("B"), Some(&1));
    assert_eq!(pair.field(&a, "name"), Ok(key("A")));
    assert_eq!(pair.field(&b, "name"), Ok(key("B")));
    assert_eq!(pair.field(&a, "answer"), Ok(Value::Number(42.0)));
    assert_eq!(pair.field(&b, "answer"), Ok(Value::Number(7.0)));
    assert_eq!(pair.field(&a, "inner"), Ok(b.clone()));
    assert_eq!(pair.field(&b, "inner"), Ok(Value::Boolean(true)));
    assert_eq!(pair.require("A"), Ok(a));
    assert_eq!(pair.require("B"), Ok(b));
}
