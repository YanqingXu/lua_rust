use lua_core::Value;

use super::model::Cache;

// ANCHOR: require_cache_lookup_starter
pub fn lookup(cache: &Cache, name: &[u8]) -> Option<Value> {
    // TODO 1: an existing false slot is a cache miss too.
    let loaded = cache.get(name);
    (loaded != Value::Nil).then_some(loaded)
}
// ANCHOR_END: require_cache_lookup_starter

// ANCHOR: require_cache_finish_starter
pub fn finish(cache: &mut Cache, name: &[u8], results: Vec<Value>) -> Value {
    // TODO 2: preserve first-nil semantics and loader-published slot contents.
    let returned = results.into_iter().next().unwrap_or(Value::Boolean(true));
    cache.set(name, returned.clone());
    returned
}
// ANCHOR_END: require_cache_finish_starter
