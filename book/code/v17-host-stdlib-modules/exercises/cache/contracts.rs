use lua_core::{ByteString, Value};

use super::model::{Cache, Design, Finish, Loader, Lookup, ModuleVm};
use super::solution;

pub fn truthy_cache_lookup(lookup: Lookup) {
    for value in [
        Value::Nil,
        Value::Boolean(false),
        Value::Boolean(true),
        Value::Number(0.0),
        Value::String(ByteString::new(&b""[..])),
    ] {
        let mut cache = Cache::default();
        cache.set(b"demo", value.clone());
        assert_eq!(
            lookup(&cache, b"demo"),
            value.is_truthy().then_some(value.clone()),
            "loaded={value}"
        );
        let mut vm = ModuleVm::new(Design {
            lookup,
            finish: solution::finish,
        });
        let loader = Loader {
            publish: None,
            results: vec![value.clone()],
        };
        vm.require(b"demo", &loader);
        vm.require(b"demo", &loader);
        assert_eq!(vm.calls, if value == Value::Boolean(false) { 2 } else { 1 });
        assert!(vm.entry_slots.iter().all(|v| *v == Value::Boolean(true)));
    }
}

pub fn first_result_commit(finish: Finish) {
    use Value::{Boolean as B, Nil as N, Number as Num};
    let empty = Value::String(ByteString::new(&b""[..]));
    let cases = [
        ("empty", None, vec![], B(true), B(true), 1),
        ("nil", None, vec![N], B(true), B(true), 1),
        ("true", None, vec![B(true)], B(true), B(true), 1),
        ("false", None, vec![B(false)], B(false), B(false), 2),
        ("zero", None, vec![Num(0.0)], Num(0.0), Num(0.0), 1),
        (
            "empty string",
            None,
            vec![empty.clone()],
            empty.clone(),
            empty,
            1,
        ),
        (
            "published",
            Some(Num(17.0)),
            vec![N],
            Num(17.0),
            Num(17.0),
            1,
        ),
        ("cleared", Some(N), vec![N], B(true), N, 2),
        (
            "published false",
            Some(B(false)),
            vec![],
            B(false),
            B(false),
            2,
        ),
        ("first nil", None, vec![N, Num(99.0)], B(true), B(true), 1),
        (
            "cleared first nil",
            Some(N),
            vec![N, Num(99.0)],
            B(true),
            N,
            2,
        ),
        (
            "override publication",
            Some(Num(17.0)),
            vec![B(false), Num(99.0)],
            B(false),
            B(false),
            2,
        ),
    ];
    // ANCHOR: require_cache_finish_contract
    for (label, publish, results, returned, slot, calls) in cases {
        let mut vm = ModuleVm::new(Design {
            lookup: solution::lookup,
            finish,
        });
        let loader = Loader { publish, results };
        assert_eq!(vm.require(b"demo", &loader), returned, "{label}: first");
        assert_eq!(vm.require(b"demo", &loader), returned, "{label}: second");
        assert_eq!(vm.loaded.get(b"demo"), slot, "{label}: slot");
        assert_eq!(vm.calls, calls, "{label}: loader calls");
        assert!(
            vm.entry_slots.iter().all(|v| *v == B(true)),
            "{label}: sentinel"
        );
    }
    // ANCHOR_END: require_cache_finish_contract
}
