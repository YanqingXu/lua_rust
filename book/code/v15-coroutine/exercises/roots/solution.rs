use crate::model::{Id, RootContext, State};

// ANCHOR: coroutine_mirror_solution
pub fn mirror_roots(state: &State) -> Vec<Id> {
    let mut roots = state.entry.iter().copied().collect::<Vec<_>>();
    for frame in &state.frames {
        roots.push(frame.function);
        roots.extend(frame.varargs.iter().copied());
        for slot in &frame.registers {
            roots.push(*slot.borrow());
        }
    }
    roots
}
// ANCHOR_END: coroutine_mirror_solution

// ANCHOR: coroutine_direct_solution
pub fn extra_direct_roots(context: &RootContext) -> Vec<Id> {
    context
        .open_upvalues
        .iter()
        .filter(|(owner, _)| context.active_frames.contains(owner))
        .map(|(_, upvalue)| *upvalue)
        .collect()
}
// ANCHOR_END: coroutine_direct_solution
