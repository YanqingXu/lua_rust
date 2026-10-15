use crate::model::{key, GlobalVm, Install};
use lua_core::Value;

pub fn single_environment(install: Install) {
    let mut vm = GlobalVm::new(install);
    let owner = vm.environment.owner_table();
    assert_eq!(vm.global("_G"), Ok(Value::Table(owner)));
    assert_eq!(vm.through_g("_G"), Ok(Value::Table(owner)));
    assert_eq!(vm.global("seed"), Ok(Value::Number(1.0)));
    assert_eq!(vm.through_g("seed"), Ok(Value::Number(1.0)));
    vm.set_global("forward", Value::Number(12.0)).unwrap();
    assert_eq!(
        vm.through_g("forward"),
        Ok(Value::Number(12.0)),
        "HashMap 写入不会更新 _G 快照"
    );
    vm.set_through_g("reverse", Value::Number(34.0)).unwrap();
    assert_eq!(vm.global("reverse"), Ok(Value::Number(34.0)));
    vm.set_global("forward", Value::Nil).unwrap();
    assert_eq!(vm.global("forward"), Ok(Value::Nil));
    assert_eq!(vm.through_g("forward"), Ok(Value::Nil));
    vm.set_through_g("reverse", Value::Nil).unwrap();
    assert_eq!(vm.global("reverse"), Ok(Value::Nil));
    assert_eq!(vm.through_g("reverse"), Ok(Value::Nil));

    let replacement = vm.self_cycle();
    vm.set_global("_G", Value::Table(replacement)).unwrap();
    vm.set_global("bare", Value::Number(77.0)).unwrap();
    assert_eq!(vm.environment.owner_table(), owner);
    assert_eq!(
        vm.heap.table_get(owner, &key("bare")),
        Ok(Value::Number(77.0))
    );
    assert_eq!(vm.through_g("bare"), Ok(Value::Nil));
    vm.set_through_g("field", Value::Number(88.0)).unwrap();
    assert_eq!(vm.global("field"), Ok(Value::Nil));
    assert_eq!(vm.through_g("field"), Ok(Value::Number(88.0)));
    vm.set_global("_G", Value::Nil).unwrap();

    let payload = vm.self_cycle();
    let garbage = vm.self_cycle();
    vm.set_global("payload", Value::Table(payload)).unwrap();
    vm.collect();
    assert_eq!(
        vm.heap.table_get(payload, &key("answer")),
        Ok(Value::Number(42.0))
    );
    assert_eq!(
        vm.heap.is_live(garbage),
        Ok(false),
        "复制到 Rust 变量的 handle 不保活自环"
    );
    assert!(vm.heap.table_get(garbage, &key("answer")).is_err());
    assert_eq!(
        vm.heap.table_get(owner, &key("bare")),
        Ok(Value::Number(77.0))
    );
    vm.set_global("payload", Value::Nil).unwrap();
    vm.collect();
    assert_eq!(vm.global("payload"), Ok(Value::Nil));
    assert_eq!(vm.heap.is_live(payload), Ok(false));
    assert!(vm.heap.table_get(payload, &key("answer")).is_err());
    assert_eq!(vm.global("_G"), Ok(Value::Nil));
    assert_eq!(vm.environment.owner_table(), owner);
    assert_eq!(vm.global("bare"), Ok(Value::Number(77.0)));
}
