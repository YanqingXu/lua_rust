use lua_core::Value;

use super::model::Cache;

// ANCHOR: require_cache_lookup_solution
pub fn lookup(cache: &Cache, name: &[u8]) -> Option<Value> {
    let loaded = cache.get(name);
    loaded.is_truthy().then_some(loaded)
}
// ANCHOR_END: require_cache_lookup_solution

// ANCHOR: require_cache_finish_solution
pub fn finish(cache: &mut Cache, name: &[u8], results: Vec<Value>) -> Value {
    let returned = results.into_iter().next().unwrap_or(Value::Nil);
    if returned != Value::Nil {
        cache.set(name, returned.clone());
        returned
    } else {
        let loaded = cache.get(name);
        if loaded == Value::Nil {
            Value::Boolean(true) // Return fallback; do not write it into cache.
        } else {
            loaded
        }
    }
}
// ANCHOR_END: require_cache_finish_solution
