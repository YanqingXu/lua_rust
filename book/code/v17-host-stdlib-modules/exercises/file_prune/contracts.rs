use crate::model::{with_fixture, Exclusive, Prune, View};
use std::fmt::Debug;

pub fn equal<T: PartialEq + Debug>(label: &str, actual: T, expected: T) -> Result<(), String> {
    if actual != expected {
        return Err(format!("{label}: expected {expected:?}, got {actual:?}"));
    }
    Ok(())
}

fn os_states(view: &View, expected: [Exclusive; 4]) -> Result<(), String> {
    if cfg!(windows) {
        equal("exclusive reopen", view.exclusive, expected)
    } else {
        equal(
            "OS evidence unavailable",
            view.exclusive,
            [Exclusive::Unsupported; 4],
        )
    }
}

pub fn incomplete_cycle_keeps_map(prune: Prune) -> Result<View, String> {
    with_fixture(|model| {
        let step = model.partial_sweep()?;
        equal("partial cycle", step.completed.is_none(), true)?;
        equal(
            "third userdata already swept",
            model.heap.is_live(model.handles[2]),
            Ok(false),
        )?;
        prune(model, &step);
        let view = model.view()?;
        equal(
            "incomplete cycle retains all entries",
            view.entries,
            [true; 4],
        )?;
        equal(
            "live tombstone retained",
            view.open,
            [true, false, true, false],
        )?;
        Ok(view)
    })
}

// ANCHOR: file_prune_contract
pub fn completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones(
    prune: Prune,
) -> Result<[View; 3], String> {
    use Exclusive::{Available as A, Blocked as B};
    with_fixture(|model| {
        let initial = model.view()?;
        equal("initial live", initial.live, [true; 4])?;
        equal(
            "initial actual userdata access",
            initial.accessible,
            [true; 4],
        )?;
        os_states(&initial, [B, A, B, A])?;
        let partial = model.partial_sweep()?;
        prune(model, &partial);
        equal(
            "do not prune incomplete cycle",
            model.view()?.entries,
            [true; 4],
        )?;
        let completed = model.finish_cycle(false)?;
        let gap = model.view()?;
        equal(
            "Heap liveness after Sweep",
            gap.live,
            [true, true, false, false],
        )?;
        equal(
            "actual userdata access",
            gap.accessible,
            [true, true, false, false],
        )?;
        equal("map before retain", gap.entries, [true; 4])?;
        os_states(&gap, [B, A, B, A])?;
        prune(model, &completed);
        let pruned = model.view()?;
        equal(
            "map after retain",
            pruned.entries,
            [true, true, false, false],
        )?;
        equal(
            "open owner after retain",
            pruned.open,
            [true, false, false, false],
        )?;
        os_states(&pruned, [B, A, A, A])?;
        model.roots.clear();
        let completed = model.finish_cycle(false)?;
        prune(model, &completed);
        let released = model.view()?;
        equal("all handles dead", released.live, [false; 4])?;
        equal("all actual accesses fail", released.accessible, [false; 4])?;
        equal("all map entries removed", released.entries, [false; 4])?;
        os_states(&released, [A; 4])?;
        Ok([gap, pruned, released])
    })
}
// ANCHOR_END: file_prune_contract

pub fn missing_prune(prune: Prune) -> Result<View, String> {
    with_fixture(|model| {
        let completed = model.finish_cycle(false)?;
        prune(model, &completed);
        let view = model.view()?;
        equal("Heap dead", view.accessible, [true, true, false, false])?;
        equal("stale keys remain", view.entries, [true; 4])?;
        equal(
            "dead userdata still owns File",
            view.open,
            [true, false, true, false],
        )?;
        use Exclusive::{Available as A, Blocked as B};
        os_states(&view, [B, A, B, A])?;
        Ok(view)
    })
}

pub fn option_is_not_liveness(prune: Prune) -> Result<View, String> {
    with_fixture(|model| {
        let completed = model.finish_cycle(false)?;
        prune(model, &completed);
        let view = model.view()?;
        equal(
            "actual userdata access",
            view.accessible,
            [true, true, false, false],
        )?;
        equal(
            "wrong predicate removes live tombstone",
            view.entries,
            [true, false, true, false],
        )?;
        equal(
            "wrong predicate keeps dead open owner",
            view.open,
            [true, false, true, false],
        )?;
        use Exclusive::{Available as A, Blocked as B};
        os_states(&view, [B, A, B, A])?;
        Ok(view)
    })
}

pub fn root_all_keys_leaks(prune: Prune) -> Result<View, String> {
    with_fixture(|model| {
        // Fixed GC-root fault: even clearing legitimate roots cannot break the map's self-rooting.
        model.roots.clear();
        let completed = model.finish_cycle(true)?;
        prune(model, &completed);
        let view = model.view()?;
        equal("artificial roots keep all userdata", view.live, [true; 4])?;
        equal("all actual accesses succeed", view.accessible, [true; 4])?;
        equal(
            "liveness retain cannot cure artificial roots",
            view.entries,
            [true; 4],
        )?;
        Ok(view)
    })
}
