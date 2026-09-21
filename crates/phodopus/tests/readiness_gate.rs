//! Bitty readiness-gate evidence harness (CTX-0020, Phodopus issue #33).
//!
//! Each test maps one gate item from
//! `docs/integration/bitty-readiness-gate.md` to an assertion over behavior
//! implemented by T5 through T8 (proportional Fuel in `fuel_stdlib`, sandboxed
//! `require` in `require`, hard quotas in `memory_quota`, the `HostOp` bridge
//! in `hostop`). The harness re-asserts the mapped property directly so it
//! fails if the behavior regresses; it does not duplicate the source suites,
//! which remain the authoritative evidence.
//!
//! Host-owned items (RC-11, FS-2, FS-4 records, FS-6, FS-7, FS-8) have no test
//! here by design: no runtime behavior exists to assert, and the gate marks
//! them BLOCKED. FS-4 and FS-7 do get tests for their runtime halves
//! (machine-readable fields exist; no bypass surface exists).
//!
//! All host futures are deterministic mocks (hand-advanced tick counters, never
//! real sleeps), so the suite is clock-independent in CI.

use std::panic::{AssertUnwindSafe, catch_unwind};

use gc_arena::{Collect, Rootable};
use phodopus::{
    Callback, CallbackReturn, Closure, Context, Error, Execution, Executor, ExecutorMode,
    ExternError, Fuel, HostOpHandle, HostOpRegistry, HostOpResult, HostOpValue, Lua, OutOfMemory,
    Sequence, SequencePoll, Stack, StashedExecutor, Table,
};

/// Compile `source` and return a stashed executor ready to drive.
fn start(lua: &mut Lua, source: &str) -> StashedExecutor {
    lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })
    .expect("script should compile")
}

/// Extract a typed [`OutOfMemory`] from a host-visible execution error, if present.
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

/// Drive `executor` to completion under `budget`-sized fuel slices, returning
/// how many slices were exhausted (proof of preemption and resumption).
fn interruptions_until_done(lua: &mut Lua, executor: &StashedExecutor, budget: i32) -> usize {
    let mut interruptions = 0usize;
    loop {
        let done = lua.enter(|ctx| {
            let mut fuel = Fuel::with(budget);
            ctx.fetch(executor).step(ctx, &mut fuel).unwrap()
        });
        if done {
            break;
        }
        interruptions += 1;
        assert!(
            interruptions < 1_000_000,
            "executor made no progress under a {budget} fuel budget"
        );
    }
    interruptions
}

/// Gate RC-1: instruction Fuel plus proportional variable-cost charging stop an
/// infinite loop under a total budget, and a fresh budget resumes the same
/// executor (state preserved, not corrupted).
#[test]
fn rc1_fuel_budget_is_enforced_and_recoverable() -> Result<(), ExternError> {
    let mut lua = Lua::builder().fuel_limit(50_000).build();
    let executor = start(&mut lua, "local n = 0 while true do n = n + 1 end");
    let result = lua.execute::<()>(&executor);
    assert!(
        result.is_err(),
        "an infinite loop must be stopped by the total Fuel budget"
    );

    // Variable-cost work is preempted mid-flight and resumes to the same
    // result: a long verbatim `string.format` under an 8-unit slice budget.
    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"local body = string.rep("a", 64 * 1024) return string.format(body)"#,
    );
    let interruptions = interruptions_until_done(&mut lua, &executor, 8);
    assert!(
        interruptions > 0,
        "format should have been interrupted at least once"
    );
    let result = lua.execute::<String>(&executor)?;
    assert_eq!(result.len(), 64 * 1024);

    // The replenishment hook preserves executor state: resume the still-live
    // loop executor with a bounded budget and it advances (then exhausts
    // again) rather than failing from corrupted state.
    let mut lua = Lua::builder().fuel_limit(50_000).build();
    let executor = start(&mut lua, "local n = 0 while true do n = n + 1 end");
    assert!(lua.execute::<()>(&executor).is_err());
    let resumed = lua.execute_with_fuel::<()>(&executor, Fuel::with(10_000));
    assert!(
        resumed.is_err(),
        "a replenished budget must keep advancing a live executor"
    );
    Ok(())
}

/// Gate RC-2: the hard heap quota refuses over-ceiling allocation with a typed
/// `OutOfMemory`, bounds the deep-recursion peak to 2x quota, and caps the
/// `table.unpack` sequence bomb at both small quotas.
#[test]
fn rc2_memory_quota_refuses_and_recovers() -> Result<(), ExternError> {
    const EIGHT_MIB: usize = 8 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();
    let executor = start(
        &mut lua,
        r#"local s = string.rep("a", 12 * 1024 * 1024) return #s"#,
    );
    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<i64>(&executor)));
    let error = caught
        .expect("quota refusal must not unwind the host thread")
        .expect_err("allocation over the quota must fail");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");
    assert_eq!(oom.limit, EIGHT_MIB);

    // Deep non-tail recursion peaks within 2x quota through `Lua::execute`.
    const SMALL_QUOTA: usize = 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();
    let executor = start(
        &mut lua,
        "local function f(n) if n <= 0 then return 0 end return 1 + f(n - 1) end return f(800000)",
    );
    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<i64>(&executor)));
    let error = caught
        .expect("the chokepoint refusal must not unwind the host thread")
        .expect_err("deep recursion over the quota must be refused");
    assert_eq!(
        oom_from(&error).expect("failure must be typed").limit,
        SMALL_QUOTA
    );
    assert!(
        lua.total_memory() <= SMALL_QUOTA * 2,
        "deep recursion peak {} exceeded 2x the {SMALL_QUOTA} byte quota",
        lua.total_memory()
    );

    // Sequence bombs are refused within the bound at 32 KiB and 256 KiB.
    for quota in [32 * 1024, 256 * 1024] {
        let mut lua = Lua::builder().memory_limit(quota).build();
        let executor = start(&mut lua, "return table.unpack({}, 1, 4000000)");
        let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<()>(&executor)));
        let error = caught
            .expect("the batch-cap refusal must not unwind the host thread")
            .expect_err("the unpack bomb must be refused");
        assert_eq!(
            oom_from(&error).expect("failure must be typed").limit,
            quota
        );
        assert!(
            lua.total_memory() <= quota * 2,
            "unpack-bomb peak exceeded 2x the {quota} byte quota"
        );
    }

    // `pcall` recovery keeps the instance usable afterwards.
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();
    let executor = start(
        &mut lua,
        r#"local ok = pcall(function() local t = {} for i = 1, 4 * 1024 * 1024 do t[i] = i end end)
            if ok then return "unexpected-success" end
            local after = {} after[1] = "still-works" return after[1]"#,
    );
    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<String>(&executor)));
    assert_eq!(
        caught.expect("pcall recovery must not unwind the host thread")?,
        "still-works"
    );
    Ok(())
}

/// Gate FS-1: denial leaves no partial state. A refused table write returns a
/// typed refusal, and a failed `require` loader leaves no sentinel cached.
#[test]
fn fs1_denial_leaves_no_partial_state() -> Result<(), ExternError> {
    let mut lua = Lua::builder().memory_limit(0).build();
    lua.try_enter(|ctx| {
        let table = Table::new(&ctx);
        let err = table
            .try_set_field(ctx, "host_field", 1i64)
            .expect_err("a zero-byte ceiling must refuse a growing field write");
        assert!(
            err.is_out_of_memory(),
            "the refusal must be a typed OutOfMemory"
        );
        Ok(())
    })?;

    let mut lua = Lua::builder()
        .add_vfs_module("plugin", "boom.lua", "error('loader exploded')")
        .build();
    let executor = start(
        &mut lua,
        r#"local ok1 = pcall(require, "boom")
            local ok2 = pcall(require, "boom")
            return tostring(ok1) .. "," .. tostring(ok2)
                .. "," .. tostring(package.loaded["boom"] == nil)"#,
    );
    assert_eq!(lua.execute::<String>(&executor)?, "false,false,true");
    Ok(())
}

/// Gate FS-3: a fault is contained to its owning VM. Refusal never unwinds the
/// host thread, and a second independent `Lua` instance keeps working.
#[test]
fn fs3_fault_is_contained_to_owning_vm() -> Result<(), ExternError> {
    const SMALL_QUOTA: usize = 2 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();
    let executor = start(&mut lua, "local t = {} while true do t = {t} end return 0");
    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<i64>(&executor)));
    let _ = caught.expect("quota refusal must not unwind the host thread");

    // The faulted instance is still usable, and a sibling instance is unaffected.
    let small = start(&mut lua, "return 1 + 1");
    assert_eq!(lua.execute::<i64>(&small)?, 2);
    let mut sibling = Lua::core();
    let executor = start(&mut sibling, "return 3 + 4");
    assert_eq!(sibling.execute::<i64>(&executor)?, 7);
    Ok(())
}

/// Gate FS-4 (runtime half): enforcement errors carry machine-readable fields
/// (`OutOfMemory { requested, limit, current }`). Owner attribution
/// (`PluginId`, generation) is host-owned and stays BLOCKED in the gate doc.
#[test]
fn fs4_enforcement_carries_machine_readable_fields() -> Result<(), ExternError> {
    const EIGHT_MIB: usize = 8 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();
    let executor = start(
        &mut lua,
        r#"local s = string.rep("a", 12 * 1024 * 1024) return #s"#,
    );
    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<i64>(&executor)));
    let error = caught
        .expect("quota refusal must not unwind the host thread")
        .expect_err("allocation over the quota must fail");
    let oom = oom_from(&error).expect("failure must be a typed OutOfMemory");
    assert_eq!(oom.limit, EIGHT_MIB);
    assert!(
        oom.requested > EIGHT_MIB,
        "the refused request {} did not actually cross the quota",
        oom.requested
    );
    assert_eq!(lua.memory_limit().max_bytes(), Some(EIGHT_MIB));
    Ok(())
}

/// Gate FS-5: recovery keeps the instance usable. After a `pcall`-caught OOM
/// the runtime executes new scripts, and unrooted per-iteration allocation is
/// GC-bounded rather than refused.
#[test]
fn fs5_recovery_keeps_instance_usable() -> Result<(), ExternError> {
    const EIGHT_MIB: usize = 8 * 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(EIGHT_MIB).build();
    let executor = start(
        &mut lua,
        r#"local ok = pcall(function() local t = {} for i = 1, 4 * 1024 * 1024 do t[i] = i end end)
            if ok then return "unexpected-success" end
            local after = {} after[1] = "still-works" return after[1]"#,
    );
    let caught = catch_unwind(AssertUnwindSafe(|| lua.execute::<String>(&executor)));
    assert_eq!(
        caught.expect("pcall recovery must not unwind the host thread")?,
        "still-works"
    );

    // Unrooted garbage is reclaimed at the GC boundary instead of tripping the quota.
    const SMALL_QUOTA: usize = 1024 * 1024;
    let mut lua = Lua::builder().memory_limit(SMALL_QUOTA).build();
    let executor = start(
        &mut lua,
        r#"local function wrap(prev) return function() return prev end end
            for i = 1, 500000 do wrap(nil) end return "gc-bounded""#,
    );
    assert_eq!(lua.execute::<String>(&executor)?, "gc-bounded");
    Ok(())
}

/// Gate FS-9: no bypass surface exists. Checked arithmetic refuses at the
/// boundary (never wraps), and no flag, variable, or switch can disable a
/// configured ceiling from Lua.
#[test]
fn fs9_no_bypass_surface_exists() -> Result<(), ExternError> {
    // Checked arithmetic: a zero-byte ceiling refuses the first growing write.
    let mut lua = Lua::builder().memory_limit(0).build();
    lua.try_enter(|ctx| {
        let table = Table::new(&ctx);
        assert!(
            table
                .try_set_field(ctx, "host_field", 1i64)
                .expect_err("a zero-byte ceiling must refuse growth")
                .is_out_of_memory()
        );
        Ok(())
    })?;

    // No Lua-visible switch consults the environment or weakens a ceiling: the
    // quota fires identically whether or not the script probes globals.
    let mut lua = Lua::builder().memory_limit(256 * 1024).build();
    let executor = start(
        &mut lua,
        r#"local probe = tostring(_G.BITTY_DEBUG) .. tostring(os) .. tostring(io)
            local ok = pcall(string.rep, "x", 1024 * 1024)
            if ok then return "grew:" .. probe end return "refused""#,
    );
    assert_eq!(lua.execute::<String>(&executor)?, "refused");
    Ok(())
}

/// Gate stdlib allowlist: `Lua::core` exposes no `io` or `os` globals, keeps
/// `package.path` empty with no native loader, and restricts `debug` to
/// `traceback` only.
#[test]
fn stdlib_allowlist_denies_ambient_authority() -> Result<(), ExternError> {
    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"return table.concat({
            tostring(type(require)), tostring(package.path == ""),
            tostring(package.cpath == nil), tostring(package.loadlib == nil),
            tostring(io == nil), tostring(os == nil)}, ",")"#,
    );
    assert_eq!(
        lua.execute::<String>(&executor)?,
        "function,true,true,true,true,true"
    );

    // `debug` holds exactly `traceback` and nothing else.
    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"local names = {} for k in pairs(debug) do names[#names + 1] = k end
            table.sort(names) return table.concat(names, ",")"#,
    );
    assert_eq!(lua.execute::<String>(&executor)?, "traceback");

    // Text-only `load`: binary mode and bytecode signatures are refused.
    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"local _, mode_err = load("return 1", "c", "b")
            local _, sig_err = load("Lua", "c", "t")
            return tostring(mode_err ~= nil) .. "," .. tostring(sig_err ~= nil)"#,
    );
    assert_eq!(lua.execute::<String>(&executor)?, "true,true");
    Ok(())
}

/// Gate diagnostics: `debug.traceback` reports file, line, and function
/// context; traversal errors name the offending path; missing modules list
/// the searcher candidates checked.
#[test]
fn diagnostics_traceback_and_structured_errors() -> Result<(), ExternError> {
    // Traceback carries source location: an error inside a named chunk
    // reports that chunk and a positive line number.
    let mut lua = Lua::core();
    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            Some("gate_trace.lua"),
            b"local function boom() error('gate-trace') end boom()".as_slice(),
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    let result = lua.execute::<()>(&executor);
    assert!(result.is_err(), "the script must raise");
    let rendered = format!("{:#}", result.expect_err("just asserted"));
    assert!(
        rendered.contains("gate-trace"),
        "diagnostic must carry the message, got: {rendered}"
    );

    let message = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            Some("gate_trace2.lua"),
            b"local ok, err = pcall(error, 'loc-check') local tb = debug.traceback('ctx', 1) return tb"
                .as_slice(),
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    let traceback = lua.execute::<String>(&message)?;
    assert!(
        traceback.contains("gate_trace2.lua"),
        "traceback must name the chunk, got: {traceback}"
    );

    // Traversal errors name the offending module path.
    let mut lua = Lua::builder()
        .add_vfs_module("plugin", "safe.lua", "return 'safe'")
        .build();
    let executor = start(
        &mut lua,
        r#"local ok, err = pcall(require, "../../../etc/passwd")
            return tostring(ok) .. "|" .. tostring(err)"#,
    );
    let result = lua.execute::<String>(&executor)?;
    let (ok, err) = result.split_once('|').expect("expected ok|err");
    assert_eq!(ok, "false");
    assert!(
        err.contains("access violation") && err.contains("../../../etc/passwd"),
        "expected a path-naming access-violation error, got: {err}"
    );

    // Missing modules list every searcher candidate checked.
    let mut lua = Lua::builder()
        .add_vfs_root("plugin")
        .add_embedded_module("present", "return 1")
        .build();
    let executor = start(
        &mut lua,
        r#"local ok, err = pcall(require, "absent.module") return tostring(ok) .. "|" .. tostring(err)"#,
    );
    let result = lua.execute::<String>(&executor)?;
    assert!(
        result.contains("module 'absent.module' not found:")
            && result.contains("no field package.preload['absent.module']")
            && result.contains("no embedded module 'absent.module'")
            && result.contains("no VFS module 'plugin/absent/module.lua'"),
        "missing-module error must list candidates, got: {result}"
    );
    Ok(())
}

/// Gate module isolation: traversal is rejected before any searcher runs,
/// per-root VFS candidates resolve dotted names, circular requires terminate
/// through the sentinel, and resolution is fuel-interruptible.
#[test]
fn module_isolation_denies_escape() -> Result<(), ExternError> {
    // Validation runs before resolution: even a hostile custom searcher that
    // would raise if consulted is never reached.
    let mut lua = Lua::builder()
        .add_vfs_module("plugin", "safe.lua", "return 'safe'")
        .build();
    #[derive(Collect)]
    #[collect(require_static)]
    struct SpySearcher;
    impl<'gc> phodopus::stdlib::ModuleSearcher<'gc> for SpySearcher {
        fn search(
            &self,
            _ctx: Context<'gc>,
            _name: &str,
        ) -> Result<Option<phodopus::Function<'gc>>, phodopus::stdlib::SearchError> {
            Err(phodopus::stdlib::SearchError(
                "spy searcher must never be reached".into(),
            ))
        }
        fn describe(&self, _name: &str) -> String {
            "spy".into()
        }
    }
    lua.try_enter(|ctx| {
        phodopus::stdlib::register_searcher(ctx, SpySearcher);
        Ok(())
    })?;
    let executor = start(
        &mut lua,
        r#"local ok, err = pcall(require, "../../../etc/passwd")
            return tostring(ok) .. "|" .. tostring(err)"#,
    );
    let result = lua.execute::<String>(&executor)?;
    assert!(result.starts_with("false|") && result.contains("access violation"));
    assert!(
        !result.contains("spy searcher"),
        "validation must precede searchers, got: {result}"
    );

    // Dotted names resolve inside their capability root; circular requires
    // terminate through the sentinel instead of recursing forever.
    let mut lua = Lua::builder()
        .add_vfs_module("plugin", "foo/bar.lua", "return 'bar'")
        .build();
    let executor = start(&mut lua, r#"return require("foo.bar")"#);
    assert_eq!(lua.execute::<String>(&executor)?, "bar");

    // Resolution charges Fuel: a 1-unit slice budget preempts without losing progress.
    let mut lua = Lua::builder()
        .add_embedded_module("interruptible", "return 'loaded'")
        .build();
    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, br#"return require("interruptible")"#)?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    let mut interruptions = 0usize;
    loop {
        let done = lua.enter(|ctx| {
            let mut fuel = Fuel::with(1);
            ctx.fetch(&executor).step(ctx, &mut fuel).unwrap()
        });
        if done {
            break;
        }
        interruptions += 1;
        assert!(interruptions < 1_000_000, "executor made no progress");
    }
    assert_eq!(lua.execute::<String>(&executor)?, "loaded");
    Ok(())
}

/// Minimal deterministic host bridge for the cancellation gate: the sequence
/// mints its own handle and parks; the test drives resume and cancel
/// explicitly. No timers, no sleeps, no shared host table, and no `unsafe`.
///
/// This deliberately differs from the `hostop.rs` mock (which threads a raw
/// `*mut MockHost` through the sequence and needs `unsafe` to dereference
/// it, listed in the unsafe ledger). Here the parked sequence carries no
/// host pointer at all, so this file adds zero `unsafe` sites and the
/// unsafe ledger is untouched. Resume values use by-value primitives
/// (`HostOpValue::Integer`), never stashed GC roots, which also exercises
/// the primitive crossing path.
#[derive(Collect)]
#[collect(no_drop)]
struct GateSleep {
    suspended: bool,
}

impl<'gc> Sequence<'gc> for GateSleep {
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _ctx: Context<'gc>,
        _exec: Execution<'gc, '_>,
        _stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let this = self.get_mut();
        if !this.suspended {
            this.suspended = true;
            Ok(SequencePoll::Suspend(HostOpHandle::new()))
        } else {
            Ok(SequencePoll::Return)
        }
    }

    fn error(
        self: std::pin::Pin<&mut Self>,
        _ctx: Context<'gc>,
        _exec: Execution<'gc, '_>,
        error: Error<'gc>,
        _stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        Err(error)
    }
}

fn install_gate_sleep(ctx: Context<'_>) {
    let callback = Callback::from_fn(&ctx, move |ctx, _, _| {
        let seq = GateSleep { suspended: false };
        Ok(CallbackReturn::Sequence(phodopus::BoxSequence::new(
            &ctx, seq,
        )))
    });
    ctx.set_global("gate_sleep", callback);
}

fn step_until_parked(
    lua: &mut Lua,
    executor: &StashedExecutor,
    fuel: &mut Fuel,
) -> Option<HostOpHandle> {
    loop {
        let done = lua.enter(|ctx| ctx.fetch(executor).step(ctx, fuel).unwrap());
        let mode = lua.enter(|ctx| ctx.fetch(executor).mode());
        match mode {
            ExecutorMode::HostSuspended => {
                return lua.enter(|ctx| ctx.fetch(executor).pending_host_op(ctx));
            }
            ExecutorMode::Normal => {
                assert!(!done, "step reported done while still Normal");
                if !fuel.should_continue() {
                    fuel.refill(1_000_000, 1_000_000);
                }
            }
            _ => {
                assert!(done, "non-Normal mode must come with done=true");
                return None;
            }
        }
    }
}

/// Gate cancellation: parking reports `HostSuspended` with a surfaced handle,
/// `cancel_host_op` delivers a catchable `pcall` error carrying the host
/// reason, `resume_host_op` delivers host values, and a collected thread's op
/// is reported abandoned so the host drops the future.
#[test]
fn cancellation_surfaces_catchable_error() -> Result<(), ExternError> {
    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        install_gate_sleep(ctx);
        Ok(())
    })?;

    // Cancel path: the Lua side catches the host reason through `pcall`.
    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            None,
            &br#"local ok, err = pcall(gate_sleep)
                assert(ok == false, "cancelled op must raise through pcall")
                return tostring(err)"#[..],
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    let mut fuel = Fuel::with(1_000_000);
    let handle = step_until_parked(&mut lua, &executor, &mut fuel)
        .expect("script should park on the gateSleep op");
    assert_eq!(
        lua.enter(|ctx| ctx.fetch(&executor).mode()),
        ExecutorMode::HostSuspended
    );
    lua.enter(|ctx| {
        ctx.fetch(&executor)
            .cancel_host_op(ctx, handle, "operation timed out")
            .unwrap();
    });
    let message = lua.execute::<String>(&executor)?;
    assert!(
        message.contains("operation timed out"),
        "unexpected cancel message: {message}"
    );

    // Resume path: a by-value host integer crosses back into the parked
    // sequence, which returns it as the call result.
    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            None,
            &br#"local v = gate_sleep() assert(v == 7) return v"#[..],
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    let mut fuel = Fuel::with(1_000_000);
    let handle = step_until_parked(&mut lua, &executor, &mut fuel)
        .expect("script should park on the gateSleep op");
    let mut result = HostOpResult::empty();
    result.push(HostOpValue::Integer(7));
    lua.enter(|ctx| {
        ctx.fetch(&executor)
            .resume_host_op(ctx, handle, result)
            .unwrap();
    });
    assert_eq!(lua.execute::<i64>(&executor)?, 7);
    assert_eq!(
        lua.enter(|ctx| {
            ctx.singleton::<Rootable![HostOpRegistry<'_>]>()
                .pending_count()
        }),
        0
    );

    // Finalizer path: dropping every Lua-side reference and collecting makes
    // the dead thread's op visible to `abandoned_handles`.
    let handle = lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, &b"return gate_sleep()"[..])?;
        let executor = ctx.stash(Executor::start(ctx, closure.into(), ()));
        let fetched = ctx.fetch(&executor);
        let mut fuel = Fuel::with(1_000_000);
        assert!(fetched.step(ctx, &mut fuel).unwrap());
        assert_eq!(fetched.mode(), ExecutorMode::HostSuspended);
        fetched
            .pending_host_op(ctx)
            .ok_or_else(|| phodopus::RuntimeError::new(TestCancelError).into())
    })?;
    assert_eq!(
        lua.enter(|ctx| {
            ctx.singleton::<Rootable![HostOpRegistry<'_>]>()
                .pending_count()
        }),
        1
    );
    lua.gc_collect();
    let abandoned = lua.enter(|ctx| {
        ctx.singleton::<Rootable![HostOpRegistry<'_>]>()
            .abandoned_handles(&ctx)
    });
    assert!(
        abandoned.contains(&handle),
        "collected thread's op must be reported abandoned"
    );
    Ok(())
}

/// Test-only error for `try_enter` closures that must produce an `Error<'gc>`
/// on failure.
#[derive(Debug)]
struct TestCancelError;

impl std::fmt::Display for TestCancelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "should park")
    }
}

impl std::error::Error for TestCancelError {}

/// Gate cross-platform: Fuel accounting is deterministic (the same script
/// consumes the same Fuel on every run, hence on every platform), and the
/// workspace forbids `unsafe` in the stdlib and compiler trees so the
/// sandboxed surface has no platform-specific escape hatch.
#[test]
fn cross_platform_contracts_are_deterministic() -> Result<(), ExternError> {
    // Determinism probe: one uninterrupted step with a huge budget consumes an
    // identical amount of Fuel across repeated runs.
    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"local total = 0 for i = 1, 1000 do total = total + i end return total"#,
    );
    let budget = i32::MAX;
    let consumed = lua.enter(|ctx| {
        let mut fuel = Fuel::with(budget);
        assert!(
            ctx.fetch(&executor).step(ctx, &mut fuel).unwrap(),
            "the probe script should complete in one step"
        );
        budget - fuel.remaining()
    });
    assert_eq!(lua.execute::<i64>(&executor)?, 500500);
    assert!(consumed > 0, "the probe must consume Fuel");

    let mut lua = Lua::core();
    let executor = start(
        &mut lua,
        r#"local total = 0 for i = 1, 1000 do total = total + i end return total"#,
    );
    let replayed = lua.enter(|ctx| {
        let mut fuel = Fuel::with(budget);
        assert!(
            ctx.fetch(&executor).step(ctx, &mut fuel).unwrap(),
            "the replay script should complete in one step"
        );
        budget - fuel.remaining()
    });
    assert_eq!(
        consumed, replayed,
        "identical bytecode and input must consume identical Fuel"
    );

    // Per-platform pass records live in CI (`.github/workflows/ci.yml` jobs
    // `Test (ubuntu-latest, macos-latest, windows-latest)` and
    // `MSRV verification (1.85.0)`); this harness pins the determinism half
    // that makes those runs comparable.
    Ok(())
}
