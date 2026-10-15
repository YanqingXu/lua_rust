use lua_core::Value;

use super::model::Cache;

pub fn never_cache(_cache: &Cache, _name: &[u8]) -> Option<Value> {
    None
}

// ANCHOR: require_cache_existence_fault
pub fn existence_only(cache: &Cache, name: &[u8]) -> Option<Value> {
    let loaded = cache.get(name);
    (loaded != Value::Nil).then_some(loaded)
}
// ANCHOR_END: require_cache_existence_fault

// ANCHOR: require_cache_publication_fault
pub fn ignore_publication(cache: &mut Cache, name: &[u8], results: Vec<Value>) -> Value {
    let returned = results
        .into_iter()
        .next()
        .filter(|value| *value != Value::Nil)
        .unwrap_or(Value::Boolean(true));
    cache.set(name, returned.clone());
    returned
}
// ANCHOR_END: require_cache_publication_fault

// ANCHOR: require_cache_first_fault
pub fn scan_non_nil(cache: &mut Cache, name: &[u8], results: Vec<Value>) -> Value {
    let returned = results
        .into_iter()
        .find(|value| *value != Value::Nil)
        .unwrap_or(Value::Boolean(true));
    cache.set(name, returned.clone());
    returned
}
// ANCHOR_END: require_cache_first_fault
