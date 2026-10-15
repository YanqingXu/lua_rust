use crate::model::{Id, RootContext, State};

pub fn entry_only(state: &State) -> Vec<Id> {
    state.entry.iter().copied().collect()
}

pub fn root_scheduler_and_all_upvalues(context: &RootContext) -> Vec<Id> {
    let mut roots = context.scheduler_handles.clone();
    roots.extend(context.open_upvalues.iter().map(|(_, upvalue)| *upvalue));
    roots
}

pub fn root_all_upvalues(context: &RootContext) -> Vec<Id> {
    context
        .open_upvalues
        .iter()
        .map(|(_, upvalue)| *upvalue)
        .collect()
}
