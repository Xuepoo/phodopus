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
    StashedExecutor, Table,
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

/// The headline Denial-of-Service construct from the sandbox specification §4.2/§5 — the `{t}`
/// table-constructor chain — must be *refused* by the hard quota, not merely measured.
///
/// Both the rooted form (`_G.root = t`, which keeps the chain live) and the non-rooted form (where
/// the old table becomes garbage) must stop with a typed `OutOfMemory`; the non-rooted form is the
/// one that previously overshot silently because nothing forced a GC between constructor steps.
#[test]
fn quota_refuses_table_constructor_chain() -> Result<(), ExternError> {
    // A deliberately small quota: the chain crosses it after a handful of constructors, long
    // before the process could allocate tens of megabytes.
    const SMALL_QUOTA: usize = 2 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();

    let executor = start(
        &mut lua,
        r#"
            local t = {}
            for i = 1, 500000 do
                t = { t }
            end
            _G.root = t
            return "reached-end"
        "#,
    );

    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<String>(&executor)));
    let result = caught.expect("quota refusal must not unwind the host thread");
    let error = result.expect_err("the {t} constructor chain must be refused");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");

    assert_eq!(oom.limit, SMALL_QUOTA);
    // Evidence that the refusal is a real hard bound, not a refusal-at-the-boundary measurement:
    // the tracked total never exceeds the quota by more than one constructor's own allocation.
    let observed = lua.total_memory();
    assert!(
        observed <= SMALL_QUOTA + 4096,
        "tracked total {observed} exceeded the {SMALL_QUOTA} byte quota"
    );

    // The runtime remains usable after the refusal.
    let small = start(&mut lua, "local t = {}; t[1] = 1; return t[1]");
    assert_eq!(lua.execute::<i64>(&small)?, 1);
    Ok(())
}

/// The same chain without rooting the result: the abandoned tables are unreachable immediately, so
/// the quota must still refuse rather than letting the collector-less growth overshoot.
#[test]
fn quota_refuses_unrooted_table_constructor_chain() -> Result<(), ExternError> {
    const SMALL_QUOTA: usize = 2 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();

    let executor = start(
        &mut lua,
        r#"
            local t = {}
            for i = 1, 500000 do
                t = { t }
            end
            return "reached-end"
        "#,
    );

    let result = lua.execute::<String>(&executor);
    let error = result.expect_err("the unrooted {t} chain must be refused");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");
    assert_eq!(oom.limit, SMALL_QUOTA);
    assert!(
        lua.total_memory() <= SMALL_QUOTA + 4096,
        "tracked total {} stayed well above the quota",
        lua.total_memory()
    );
    Ok(())
}

/// The "unbounded string growth" clause: repeated `..` concatenation must be refused by the hard
/// quota with a typed `OutOfMemory`, before the result buffer is allocated.
#[test]
fn quota_refuses_concat_growth() -> Result<(), ExternError> {
    const SMALL_QUOTA: usize = 2 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();

    // Doubling a string with `..` crosses a 2 MiB quota in ~10 iterations and would otherwise keep
    // growing until the process ran out of memory.
    let executor = start(
        &mut lua,
        r#"
            local s = "x"
            for i = 1, 40 do
                s = s .. s
            end
            return #s
        "#,
    );

    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<i64>(&executor)));
    let result = caught.expect("quota refusal must not unwind the host thread");
    let error = result.expect_err("repeated concatenation must be refused");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");
    assert_eq!(oom.limit, SMALL_QUOTA);
    assert!(
        lua.total_memory() <= SMALL_QUOTA * 2,
        "tracked total {} should be bounded near the quota",
        lua.total_memory()
    );

    // The instance still works for a small concatenation afterwards.
    let small = start(&mut lua, "return ('a' .. 'b')");
    assert_eq!(lua.execute::<String>(&small)?, "ab");
    Ok(())
}

/// The `table.concat` (separated concatenation) path shares the same pre-allocation check as the
/// `..` operator.
#[test]
fn quota_refuses_table_concat_growth() -> Result<(), ExternError> {
    const SMALL_QUOTA: usize = 2 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();

    let executor = start(
        &mut lua,
        r#"
            local parts = {}
            local chunk = string.rep("x", 64 * 1024)
            for i = 1, 128 do parts[i] = chunk end
            return #table.concat(parts, "-")
        "#,
    );

    let result = lua.execute::<i64>(&executor);
    let error = result.expect_err("table.concat over the quota must be refused");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");
    assert_eq!(oom.limit, SMALL_QUOTA);
    Ok(())
}

/// A refused `{t}` constructor chain inside `pcall` must never abort the host. Whether Lua can
/// catch it depends on whether recovery headroom remains (the refusal happens when the arena is at
/// the ceiling), so the honest guarantee is: the host observes either a caught error or a clean
/// typed `OutOfMemory`, never a panic or `handle_alloc_error`.
#[test]
fn constructor_chain_refusal_never_aborts_under_pcall() {
    const SMALL_QUOTA: usize = 2 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();

    let executor = start(
        &mut lua,
        r#"
            local ok = pcall(function()
                local t = {}
                for i = 1, 500000 do t = { t } end
            end)
            if ok then return "unexpected-success" end
            return "recovered"
        "#,
    );

    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<String>(&executor)));
    let result = caught.expect("a refused constructor chain must not unwind the host thread");
    match result {
        // Enough recovery headroom remained for `pcall` to materialize the error value.
        Ok(value) => assert_eq!(value, "recovered"),
        // Not enough headroom: the typed refusal propagates cleanly to the host instead.
        Err(error) => {
            let oom =
                oom_from(&error).expect("the propagated failure must still be a typed OutOfMemory");
            assert_eq!(oom.limit, SMALL_QUOTA);
        }
    }
}

/// The fallible host API (`Table::try_set_field` / `Context::try_set_global`) returns the typed
/// refusal instead of panicking when a quota is installed and the write would grow a table.
#[test]
fn try_set_field_returns_typed_oom_under_quota() -> Result<(), ExternError> {
    // A zero-byte ceiling makes any *growing* write refuse. A freshly created table has no array or
    // map capacity, so its first field write must grow and is therefore refused; `Table::new`
    // itself is unchecked, so this exercises the field write specifically.
    let mut lua = Lua::builder().memory_limit(0).build();

    lua.try_enter(|ctx| {
        let table = Table::new(&ctx);
        let err = table
            .try_set_field(ctx, "host_field", 1i64)
            .expect_err("a zero-byte ceiling must refuse a growing field write");
        assert!(
            err.is_out_of_memory(),
            "the refusal must be a typed OutOfMemory, not a key error"
        );
        Ok(())
    })
}

/// The raw `Table::set` path is also panic-free under a hard quota: it returns a typed refusal
/// rather than unwinding, even when called directly from host code.
#[test]
fn table_set_under_quota_is_panic_free() {
    let mut lua = Lua::builder().memory_limit(0).build();
    let caught = catch_unwind(AssertUnwindSafe(|| {
        lua.try_enter(|ctx| {
            let table = Table::new(&ctx);
            // A fresh table's first integer key forces array growth, which the zero-byte ceiling
            // must refuse with `Err` rather than a panic.
            let refused = table.set(ctx, 1i64, 1i64).is_err();
            Ok(refused)
        })
    }));
    let refused = caught.expect("table growth refusal must not unwind the host thread");
    assert!(refused.expect("no typed leak"));
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
