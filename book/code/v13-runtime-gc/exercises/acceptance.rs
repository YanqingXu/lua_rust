mod contracts;
mod starter;

#[test]
#[ignore = "学习者修改 cycle_starter.lua 后显式运行"]
fn two_table_cycle_loses_its_last_global_root() {
    contracts::two_table_cycle(include_str!("cycle_starter.lua"));
}

#[test]
#[ignore = "学习者实现 starter.rs 的 pin / unpin 后显式运行"]
fn host_registry_lifetime() {
    contracts::host_registry_lifetime(starter::pin, starter::unpin);
}

#[test]
#[ignore = "学习者实现 starter.rs 的 restore_scope_exit 后显式运行"]
fn close_before_register_reuse() {
    contracts::close_before_register_reuse(starter::restore_scope_exit);
}

#[test]
#[ignore = "学习者实现 starter.rs 的 primitive_roots 后显式运行"]
fn primitive_metatable_root_lifetime() {
    contracts::primitive_metatable_root_lifetime(starter::primitive_roots);
}
