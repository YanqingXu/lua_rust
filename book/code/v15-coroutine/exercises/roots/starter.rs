use crate::model::{Id, RootContext, State};

// ANCHOR: coroutine_mirror_starter
pub fn mirror_roots(state: &State) -> Vec<Id> {
    // TODO 1：保留 entry，并镜像每个暂停 frame 的 function、varargs、slot 当前值。
    state.entry.iter().copied().collect()
}
// ANCHOR_END: coroutine_mirror_starter

// ANCHOR: coroutine_direct_starter
pub fn extra_direct_roots(context: &RootContext) -> Vec<Id> {
    // TODO 2：scheduler 只是索引；open upvalue 只在 owner 帧 active 时直接作为 root。
    let mut roots = context.scheduler_handles.clone();
    roots.extend(context.open_upvalues.iter().map(|(_, upvalue)| *upvalue));
    roots
}
// ANCHOR_END: coroutine_direct_starter
