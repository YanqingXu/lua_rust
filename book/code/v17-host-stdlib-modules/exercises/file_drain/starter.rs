use crate::model::Owner;

// ANCHOR: file_drain_starter
pub fn shutdown(owner: &mut Owner) {
    // TODO：在关闭 Heap 前，显式清空宿主 files owner；None 不应重复释放。
    owner.shutdown_heap();
}
// ANCHOR_END: file_drain_starter
