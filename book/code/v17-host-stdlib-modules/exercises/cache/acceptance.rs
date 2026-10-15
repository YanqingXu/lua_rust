mod contracts;
mod model;
mod solution;
mod starter;

#[test]
#[ignore = "TODO 1: only truthy loaded values are cache hits"]
fn cache_hit_requires_a_truthy_value() {
    contracts::truthy_cache_lookup(starter::lookup);
}

#[test]
#[ignore = "TODO 2: commit only the first result and preserve first-nil slot semantics"]
fn require_success_commits_only_the_first_result() {
    contracts::first_result_commit(starter::finish);
}
