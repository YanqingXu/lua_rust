use lua_core::allocator::{AllocationSite, ManagedAllocationError};
use lua_core::heap::Heap;
use lua_core::table::Table;

#[test]
fn gc_object_failure_is_transactional_and_retryable() {
    let mut heap = Heap::new();
    let before = heap.allocator_stats();

    heap.inject_allocator_failure_after(AllocationSite::GcObject, 0);
    let error = heap
        .collector_mut()
        .try_create(Table::new())
        .expect_err("selected GC object checkpoint must fail");
    assert!(matches!(
        error,
        ManagedAllocationError::Injected {
            site: AllocationSite::GcObject,
            ..
        }
    ));
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.accounted_bytes(), 0);
    assert_eq!(heap.allocator_stats(), before);

    let table = heap
        .collector_mut()
        .try_create(Table::new())
        .expect("one-shot failure permits retry");
    assert!(heap.collector().contains_registered(table));
    assert_eq!(heap.object_count(), 1);

    let report = heap.destroy_all();
    assert_eq!(report.destroyed_objects(), 1);
    assert_eq!(heap.allocator_stats().live_bytes, 0);
}

#[test]
fn publication_root_failure_leaves_no_object_or_temporary_root() {
    let mut heap = Heap::new();
    let before = heap.allocator_stats();

    heap.inject_allocator_failure_after(AllocationSite::PublicationRoot, 0);
    let result = heap
        .collector_mut()
        .with_publication(|transaction| transaction.try_alloc(Table::new()).map(|_| ()));
    assert!(matches!(
        result,
        Err(ManagedAllocationError::Injected {
            site: AllocationSite::PublicationRoot,
            ..
        })
    ));
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.collector().temporary_root_count(), 0);
    assert_eq!(heap.allocator_stats(), before);

    let published = heap.collector_mut().with_publication(|transaction| {
        let table = transaction
            .try_alloc(Table::new())
            .expect("one-shot failure permits publication retry");
        transaction
            .publish_as_explicit_root(table)
            .expect("retry allocation remains registered")
    });
    assert!(heap.collector().is_root(published));
    assert_eq!(heap.collector().temporary_root_count(), 0);

    heap.destroy_all();
    assert_eq!(heap.allocator_stats().live_bytes, 0);
}

#[test]
fn string_key_and_gc_object_failures_do_not_publish_partial_interns() {
    let mut heap = Heap::new();

    for site in [AllocationSite::StringPoolKey, AllocationSite::GcObject] {
        let before = heap.allocator_stats();
        let objects_before = heap.object_count();
        let strings_before = heap.interned_string_count();
        heap.inject_allocator_failure_after(site, 0);

        let error = heap
            .with_parts_mut(|collector, strings| {
                strings.try_intern_bytes(collector, b"fault-matrix")
            })
            .expect_err("selected string construction checkpoint must fail");
        assert!(matches!(
            error,
            ManagedAllocationError::Injected {
                site: failed_site,
                ..
            } if failed_site == site
        ));
        assert_eq!(heap.object_count(), objects_before);
        assert_eq!(heap.interned_string_count(), strings_before);
        assert_eq!(heap.allocator_stats(), before);
    }

    let string = heap
        .with_parts_mut(|collector, strings| strings.try_intern_bytes(collector, b"fault-matrix"))
        .expect("all one-shot failures permit a clean retry");
    assert!(heap.collector().contains_registered(string));
    assert_eq!(heap.interned_string_count(), 1);

    heap.destroy_all();
    assert_eq!(heap.allocator_stats().live_bytes, 0);
}
