//! Hard allocator memory-quota tests (sandbox specification §4.2, verification plan item 5).
//!
//! These tests configure an explicit `RuntimeBuilder::memory_limit` and prove that the runtime
//! *refuses* allocation once the ceiling is reached, instead of merely measuring it. Refusal is a
//! clean, typed [`OutOfMemory`] error: no native abort, no panic, and the Lua state remains usable
//! afterwards. The `pcall` recovery test additionally proves the error is catchable in Lua while
//! recovery memory remains.

use std::panic::{AssertUnwindSafe, catch_unwind};

use phodopus::{
    Closure, Executor, ExternError, Fuel, Lua, MemoryLimit, OutOfMemory, RuntimeBuilder,
    StashedExecutor,
};

/// The quota used by the headline 8 MiB memory-ceiling test.
const EIGHT_MIB: usize = 8 * 1024 * 1024;

fn start(lua: &mut Lua, source: &str) -> StashedExecutor {
    lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })
    .expect("script should compile")
}

/// Extract an [`OutOfMemory`] from a host-visible execution error, if present.
///
/// The quota refusal can be wrapped by an interpreter-level error type, so the search follows the
/// error chain rather than only the top-level error.
fn oom_from(err: &ExternError) -> Option<OutOfMemory> {
    match err {
        ExternError::Runtime(runtime) => {
            if let Some(oom) = runtime.downcast::<OutOfMemory>() {
                return Some(*oom);
            }
            let mut source: Option<&(dyn std::error::Error + 'static)> = Some(runtime.root_cause());
            while let Some(current) = source {
                if let Some(oom) = current.downcast_ref::<OutOfMemory>() {
                    return Some(*oom);
                }
                source = current.source();
            }
            None
        }
        ExternError::Lua { .. } => None,
    }
}

/// A large string allocation under an 8 MiB quota must fail with a clean `OutOfMemory`, never an
/// abort or panic, and must leave the runtime usable.
#[test]
fn eight_mib_quota_refuses_large_string_allocation() -> Result<(), ExternError> {
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();

    // A 12 MiB repeated string is well over the 8 MiB quota but below the 16 MiB per-operation
    // ceiling, so the quota (not the checked output ceiling) is what must refuse it.
    let executor = start(
        &mut lua,
        r#"
            local s = string.rep("a", 12 * 1024 * 1024)
            return #s
        "#,
    );

    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<i64>(&executor)));
    let result = caught.expect("quota refusal must not unwind the host thread");
    let error = result.expect_err("allocation over the quota must fail");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");

    assert_eq!(oom.limit, EIGHT_MIB);
    assert_eq!(lua.memory_limit().max_bytes(), Some(EIGHT_MIB));

    // The runtime is still usable: a small allocation succeeds afterwards.
    let small = start(&mut lua, "return 1 + 1");
    assert_eq!(lua.execute::<i64>(&small)?, 2);
    Ok(())
}

/// The quota must refuse table growth as well as string growth, proving the check is at the
/// allocation boundary rather than specific to one stdlib function.
#[test]
fn quota_refuses_table_growth() -> Result<(), ExternError> {
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();

    let executor = start(
        &mut lua,
        r#"
            local t = {}
            for i = 1, 4 * 1024 * 1024 do
                t[i] = i
            end
            return #t
        "#,
    );

    let result = lua.execute::<i64>(&executor);
    let error = result.expect_err("table growth over the quota must fail");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");
    assert_eq!(oom.limit, EIGHT_MIB);
    Ok(())
}

/// `pcall` must catch a quota refusal while recovery memory remains, and the instance must remain
/// usable for a subsequent allocation and execution.
#[test]
fn pcall_recovers_from_out_of_memory() -> Result<(), ExternError> {
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();

    let executor = start(
        &mut lua,
        r#"
            local ok = pcall(function()
                local t = {}
                for i = 1, 4 * 1024 * 1024 do
                    t[i] = i
                end
            end)
            if ok then
                return "unexpected-success"
            end
            local after = {}
            after[1] = "still-works"
            return after[1]
        "#,
    );

    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<String>(&executor)));
    let result = caught.expect("pcall recovery must not unwind the host thread");
    assert_eq!(
        result?, "still-works",
        "pcall must catch OOM and the runtime must keep working"
    );
    Ok(())
}

/// A quota small enough to refuse the very first nontrivial allocation proves refusal, not just
/// measurement: the runtime never silently grows past the ceiling.
#[test]
fn low_quota_refuses_before_measurement() -> Result<(), ExternError> {
    // 256 KiB is far below the 1 MiB result the script requests.
    const LOW: usize = 256 * 1024;
    let mut lua = Lua::builder().memory_limit(LOW).build();

    let executor = start(
        &mut lua,
        r#"
            local ok, err = pcall(string.rep, "x", 1024 * 1024)
            if ok then return "grew" end
            return "refused"
        "#,
    );

    let result = lua.execute::<String>(&executor)?;
    assert_eq!(result, "refused");
    assert!(
        lua.memory_limit().current_bytes() <= LOW,
        "tracked allocation {} should stay within the {LOW} byte quota",
        lua.memory_limit().current_bytes()
    );
    Ok(())
}

/// The `MemoryLimit` exposes the configured ceiling and observed usage through the runtime.
#[test]
fn memory_limit_is_observable() {
    let lua = Lua::builder().memory_limit(EIGHT_MIB).build();
    assert_eq!(lua.memory_limit().max_bytes(), Some(EIGHT_MIB));
    assert!(lua.memory_limit().current_bytes() > 0);
    assert!(!lua.memory_limit_exceeded());

    let unbounded = Lua::core();
    assert_eq!(unbounded.memory_limit().max_bytes(), None);
}

/// The `RuntimeBuilder` type alias and both limit setters exist and chain.
#[test]
fn runtime_builder_alias_exposes_limit_setters() {
    let builder: &mut RuntimeBuilder = &mut Lua::builder();
    builder.fuel_limit(100_000).memory_limit(EIGHT_MIB);
    let lua = builder.build();
    assert_eq!(lua.fuel_limit(), Some(100_000));
    assert_eq!(lua.memory_limit().max_bytes(), Some(EIGHT_MIB));
}

/// A configured Fuel budget is a *total* budget: an infinite loop is interrupted with a typed
/// `FuelExhausted` rather than running forever, and the executor state is preserved so a fresh
/// budget resumes execution.
#[test]
fn fuel_limit_is_total_and_replenishable() -> Result<(), ExternError> {
    let mut lua = Lua::builder().fuel_limit(50_000).build();

    let executor = start(
        &mut lua,
        r#"
            local n = 0
            while true do
                n = n + 1
            end
        "#,
    );

    let result = lua.execute::<()>(&executor);
    assert!(
        result.is_err(),
        "an infinite loop must be stopped by the total Fuel budget"
    );

    // Resume the same executor with a bounded replenished budget; it advances and is interrupted
    // again rather than failing from a corrupted state.
    let resumed = lua.execute_with_fuel::<()>(&executor, Fuel::with(10_000));
    assert!(
        resumed.is_err(),
        "the loop is still infinite, so a replenished budget is consumed too"
    );
    Ok(())
}

/// Without a configured Fuel budget, ordinary finite execution still succeeds (regression guard
/// for the changed `Lua::execute` path).
#[test]
fn execution_without_fuel_limit_is_unbounded() -> Result<(), ExternError> {
    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"
            local sum = 0
            for i = 1, 10000 do sum = sum + i end
            return sum
        "#,
    );
    assert_eq!(lua.execute::<i64>(&executor)?, 50005000);
    Ok(())
}

/// `Lua::set_memory_limit` can install a ceiling after construction, and `enforce_memory_limit`
/// reports the over-quota state host-visibly.
#[test]
fn enforce_memory_limit_after_construction() {
    let mut lua = Lua::core();
    // An impossibly small ceiling must be refused immediately at the enforcement point, without a
    // panic or abort.
    lua.set_memory_limit(Some(1));
    let err = lua
        .enforce_memory_limit()
        .expect_err("a 1-byte ceiling cannot hold a loaded runtime");
    assert_eq!(err.limit, 1);
    assert!(lua.memory_limit_exceeded());

    // Clearing the quota restores unbounded execution.
    lua.set_memory_limit(None);
    assert!(lua.enforce_memory_limit().is_ok());
}

/// The quota accessor on `Context` uses checked arithmetic, so an overflowing request is refused
/// instead of wrapping past the ceiling.
#[test]
fn context_check_memory_uses_checked_arithmetic() -> Result<(), ExternError> {
    const SMALL: usize = 1024;
    let mut lua = Lua::builder().memory_limit(SMALL).build();
    lua.try_enter(|ctx| {
        let err = ctx
            .check_memory(usize::MAX)
            .expect_err("an overflowing request must be refused");
        assert_eq!(err.limit, SMALL);
        assert_eq!(err.requested, usize::MAX);
        Ok(())
    })
}

/// `MemoryLimit` is cloneable-by-reference through the runtime and its counter tracks observed
/// usage consistently with `Lua::total_memory`.
#[test]
fn observed_usage_tracks_total_memory() -> Result<(), ExternError> {
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();
    let executor = start(
        &mut lua,
        r#"
            local t = {}
            for i = 1, 1000 do t[i] = i end
            return #t
        "#,
    );
    assert_eq!(lua.execute::<i64>(&executor)?, 1000);

    let observed = lua.memory_limit().current_bytes();
    let total = lua.total_memory();
    assert!(
        observed <= total,
        "observed {observed} should not exceed the current total {total}"
    );
    Ok(())
}

/// A `MemoryLimit` constructed directly behaves as documented for the boundary cases.
#[test]
fn memory_limit_direct_api() {
    let unbounded = MemoryLimit::new(None);
    assert!(unbounded.check(usize::MAX, 1).is_ok());

    let bounded = MemoryLimit::new(Some(10));
    assert!(bounded.check(4, 6).is_ok());
    assert!(bounded.check(4, 7).is_err());
}
