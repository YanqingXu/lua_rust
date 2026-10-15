use lua_compiler::compile;
use lua_core::{ByteString, Heap, HeapError, Instruction, Opcode, Proto, TableRef, Value};
use lua_vm::Vm;

pub fn two_table_cycle(source: &str) {
    let proto = compile(source.as_bytes()).unwrap();
    assert_eq!(
        Vm::new().execute(&proto),
        Ok(Value::Boolean(true)),
        "无 root 的双表环应回收；global 保留时应存活；删除 global 后应再次回收"
    );
}

pub fn host_registry_lifetime(
    pin: fn(&mut Vm, Value, Value) -> Result<(), HeapError>,
    unpin: fn(&mut Vm, Value) -> Result<(), HeapError>,
) {
    let proto = compile(b"return { answer = 42 }").unwrap();
    let key = Value::String(ByteString::new(&b"host-lease"[..]));
    let mut vm = Vm::new();

    // 负例：宿主变量及其 clone 仍在 Rust 栈上，却没有进入 VM roots。
    let unrooted = vm.execute(&proto).unwrap();
    let host_alias = unrooted.clone();
    vm.collect_garbage().unwrap();
    assert!(matches!(
        vm.registry_set(key.clone(), host_alias),
        Err(HeapError::StaleHandle { .. })
    ));

    let rooted = vm.execute(&proto).unwrap();
    let host_alias = rooted.clone();
    pin(&mut vm, key.clone(), rooted.clone()).unwrap();
    vm.collect_garbage().unwrap();
    assert_eq!(
        vm.registry_get(&key),
        Ok(rooted),
        "pin 必须在第一次 collection 之前建立 registry root"
    );

    unpin(&mut vm, key.clone()).unwrap();
    vm.collect_garbage().unwrap();
    assert_eq!(
        vm.registry_get(&key),
        Ok(Value::Nil),
        "unpin 必须删除条目；宿主仍持有 alias 不应阻止回收"
    );
    assert!(matches!(
        vm.registry_set(key.clone(), host_alias),
        Err(HeapError::StaleHandle { .. })
    ));

    // 拒绝失效值后，registry 本身仍可继续使用。
    vm.registry_set(key.clone(), Value::Number(42.0)).unwrap();
    assert_eq!(vm.registry_get(&key), Ok(Value::Number(42.0)));
}

pub fn close_before_register_reuse(restore: fn(&Proto, &mut Proto)) {
    let original = compile(
        b"local saved\n\
          do\n\
            local captured = 10\n\
            saved = function() return captured end\n\
          end\n\
          local reused = 99\n\
          return saved()",
    )
    .unwrap();
    assert_eq!(Vm::new().execute(&original), Ok(Value::Number(10.0)));
    let mut faulty = original.clone();
    let mut injected = 0;
    for instruction in &mut faulty.code {
        if instruction.opcode() == Ok(Opcode::Close) {
            // Jump 0 只跳到下一条：保留所有 instruction index。
            *instruction = Instruction::asbx(Opcode::Jump, 0, 0);
            injected += 1;
        }
    }
    assert!(injected >= 1, "反例必须真正经过 Close");
    assert_eq!(
        Vm::new().execute(&faulty),
        Ok(Value::Number(99.0)),
        "故障必须暴露寄存器复用污染，否则不是有效反例"
    );
    restore(&original, &mut faulty);
    assert_eq!(faulty.code.len(), original.code.len());
    assert_eq!(
        Vm::new().execute(&faulty),
        Ok(Value::Number(10.0)),
        "应先关闭 capture，再允许寄存器复用；延迟到函数 Return 太晚"
    );
}

pub fn primitive_metatable_root_lifetime(roots: fn(&[TableRef]) -> Vec<Value>) {
    // 这是 Vm.type_metatables 根收集的小模型，不声称它是完整 VM roots。
    let mut heap = Heap::new();
    let mt = heap.allocate_table().unwrap();
    let defaults = heap.allocate_table().unwrap();
    let unrelated = heap.allocate_table().unwrap();
    heap.table_set(mt, Value::Number(1.0), Value::Table(defaults))
        .unwrap();
    heap.table_set(defaults, Value::Number(1.0), Value::Number(42.0))
        .unwrap();

    let type_metatables = [mt];
    heap.collect(&roots(&type_metatables), &[]).unwrap();
    assert_eq!(
        heap.table_get(mt, &Value::Number(1.0)),
        Ok(Value::Table(defaults)),
        "primitive metatable root 必须能追踪到其间接 child"
    );
    assert_eq!(heap.is_live(unrelated), Ok(false));

    let removed = heap.collect(&roots(&[]), &[]).unwrap();
    assert_eq!((removed.collected, removed.remaining), (2, 0));
    assert!(matches!(
        heap.table_get(mt, &Value::Number(1.0)),
        Err(HeapError::StaleHandle { .. })
    ));
}
