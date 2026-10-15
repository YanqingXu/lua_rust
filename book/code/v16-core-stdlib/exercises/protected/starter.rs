use crate::model::Outcome;
use lua_core::Value;

// ANCHOR: protected_pcall_starter
pub fn pack_pcall(outcome: Outcome) -> Vec<Value> {
    // TODO 1：成功要保留完整结果 Vec，含中间/尾部 nil；失败才变成 false/error text。
    crate::faults::first_result_only(outcome)
}
// ANCHOR_END: protected_pcall_starter

// ANCHOR: protected_xpcall_starter
pub fn pack_xpcall(outcome: Outcome, handler: &mut dyn FnMut(Value) -> Outcome) -> Vec<Value> {
    // TODO 2：仅 body 失败时调用 handler 一次；结果只取第一个，无结果补 nil。
    // handler 自身失败时返回其 VmError 文本；不要覆盖成功结果或展开全部 handler 返回值。
    crate::faults::always_handle_and_forward_all(outcome, handler)
}
// ANCHOR_END: protected_xpcall_starter
