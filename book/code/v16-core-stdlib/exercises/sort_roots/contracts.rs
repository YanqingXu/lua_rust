use crate::model::{key, Scenario, Scope, SortError, SortVm};
use lua_core::Value;

pub fn snapshot_lifetime_and_cleanup(scope: Scope) {
    let mut vm = SortVm::new(Scenario::ClearAndCollect);
    let prefix = vm.temporary_roots.clone();
    assert_eq!(
        vm.run_sort(scope),
        Ok(()),
        "snapshot 的其他对象不能在 comparator GC 中失活"
    );
    assert_eq!(vm.source_ranks(), Ok(vec![1.0, 2.0, 3.0, 4.0]));
    assert_eq!(vm.temporary_roots, prefix);
    assert_eq!(
        vm.heap.table_get(vm.prefix, &key("sentinel")),
        Ok(Value::Number(900.0))
    );
    for item in &vm.items {
        assert!(vm.heap.table_get(*item, &key("rank")).is_ok());
    }
    vm.clear_source().unwrap();
    vm.collect().unwrap();
    for item in &vm.items {
        assert_eq!(vm.heap.is_live(*item), Ok(false));
        assert!(vm.heap.table_get(*item, &key("rank")).is_err());
    }

    let mut failing = SortVm::new(Scenario::ErrorAfterMutation);
    let prefix = failing.temporary_roots.clone();
    assert_eq!(failing.run_sort(scope), Err(SortError::Comparator));
    assert_eq!(failing.comparisons, 2);
    assert_eq!(
        failing.temporary_roots, prefix,
        "Err 也要恢复精确前缀，不能 ? 早退或 clear()"
    );
    assert_eq!(
        failing.heap.table_get(failing.source, &Value::Number(1.0)),
        Ok(Value::Number(99.0)),
        "callback mutation 不回滚"
    );
    failing.collect().unwrap();
    assert_eq!(failing.heap.is_live(failing.items[0]), Ok(false));
    assert!(failing
        .heap
        .table_get(failing.items[0], &key("rank"))
        .is_err());
    assert_eq!(
        failing.heap.table_get(failing.prefix, &key("sentinel")),
        Ok(Value::Number(900.0))
    );
}
