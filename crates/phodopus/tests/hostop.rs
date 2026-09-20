//! Typed `HostOp` host-async suspension bridge tests (Phase 4,
//! `docs/specifications/async-trampoline.md` verification plan).
//!
//! All host futures are deterministic mocks — a hand-advanced tick counter, never a real sleep —
//! so the suite is clock-independent in CI. Each test drives the trampoline explicitly: step until
//! `ExecutorMode::HostSuspended`, resolve the op via `resume_host_op` / `cancel_host_op`, step to
//! completion.
//!
//! Covered:
//!
//! 1. Mock-async-timer: script suspends, host advances the mock clock, timer completes, the Lua
//!    coroutine resumes with the correct values.
//! 2. Cancellation: host cancels; Lua `pcall` catches the error.
//! 3. Burst: 100 independent suspended coroutines all wake and complete non-blocking.
//! 4. Fuel/quota accounting: the suspended coroutine burns no fuel while parked (remaining fuel is
//!    unchanged across parked steps) and resumes correctly through the quota chokepoint.

use std::{cell::Cell, collections::HashMap, pin::Pin, rc::Rc};

use gc_arena::{Collect, Rootable};
use phodopus::{
    Callback, CallbackReturn, Closure, Context, Error, Execution, Executor, ExecutorMode,
    ExternError, Fuel, HostOpHandle, HostOpRegistry, HostOpResult, HostOpValue, Lua, Sequence,
    SequencePoll, Stack, StashedValue, Variadic,
};

/// Test-only error for `try_enter` closures that must produce an `Error<'gc>` on failure.
#[derive(Debug)]
struct TestError;

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "should park")
    }
}

impl std::error::Error for TestError {}

/// A deterministic host-side async bridge: Rust callbacks register a "timer" (ticks remaining +
/// stashed wake value); tests advance the mock clock and resolve due ops through the executor.
#[derive(Default)]
struct MockHost {
    next: u64,
    timers: HashMap<u64, MockTimer>,
}

struct MockTimer {
    remaining: u32,
    value: StashedValue,
}

impl MockHost {
    fn register(&mut self, ticks: u32, value: StashedValue) -> HostOpHandle {
        self.next += 1;
        let handle = HostOpHandle::from_raw(self.next);
        self.timers.insert(
            self.next,
            MockTimer {
                remaining: ticks,
                value,
            },
        );
        handle
    }

    /// Advance the mock clock one tick; returns the handles whose timers fired.
    fn tick(&mut self) -> Vec<(HostOpHandle, StashedValue)> {
        let mut fired = Vec::new();
        for (raw, timer) in self.timers.iter_mut() {
            if timer.remaining > 0 {
                timer.remaining -= 1;
            }
            if timer.remaining == 0 {
                fired.push((HostOpHandle::from_raw(*raw), timer.value.clone()));
            }
        }
        for (handle, _) in &fired {
            self.timers.remove(&handle.raw());
        }
        fired
    }

    fn cancel(&mut self, handle: HostOpHandle) {
        self.timers.remove(&handle.raw());
    }
}

/// A `Sequence` that suspends once on a mock host timer, then returns the wake value.
///
/// The suspending poll roots the wake value as a [`StashedValue`] in the mock host (never held as
/// a `Gc` by the mock future), registers the mock timer, and returns `SequencePoll::Suspend`.
/// The suspending stack is truncated to `bottom` on resume, so the resume poll sees exactly the
/// host's return values (placed by `resume_host_op` starting at `bottom`).
#[derive(Collect)]
#[collect(no_drop)]
struct MockSleep {
    #[collect(require_static)]
    host: Rc<Cell<*mut MockHost>>,
    ticks: u32,
    #[collect(require_static)]
    value: StashedValue,
    suspended: bool,
}

impl<'gc> Sequence<'gc> for MockSleep {
    fn poll(
        self: Pin<&mut Self>,
        _ctx: Context<'gc>,
        _exec: Execution<'gc, '_>,
        _stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        let this = self.get_mut();
        if !this.suspended {
            this.suspended = true;
            // SAFETY: the `*mut MockHost` points at the test's stack-owned `MockHost`, which
            // outlives the `Lua` instance driving this sequence; the `Rc<Cell<...>>` handle is
            // test-only scaffolding (real hosts own their future table directly).
            let host = unsafe { &mut *this.host.get() };
            let handle = host.register(this.ticks, this.value.clone());
            Ok(SequencePoll::Suspend(handle))
        } else {
            // Resumed: the host's return values were placed on the stack by `resume_host_op`
            // (the suspending stack was truncated to `bottom`, so only host values are here).
            Ok(SequencePoll::Return)
        }
    }

    fn error(
        self: Pin<&mut Self>,
        _ctx: Context<'gc>,
        _exec: Execution<'gc, '_>,
        error: Error<'gc>,
        _stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        // Cancellation (or any error) propagates to Lua `pcall` handlers.
        Err(error)
    }
}

/// Install a `mock_sleep(ticks, wake_value)` global backed by `host`.
fn install_mock_sleep(ctx: Context<'_>, host: Rc<Cell<*mut MockHost>>) {
    let callback = Callback::from_fn_with(&ctx, host, move |host, ctx, _, mut stack| {
        let (ticks, wake): (i32, phodopus::Value) = stack.consume(ctx)?;
        let value: StashedValue = ctx.stash(wake);
        // Keep the wake value on the sequence stack is unnecessary; it travels in the mock host
        // as a stashed root and returns via `resume_host_op`.
        let seq = MockSleep {
            host: host.clone(),
            ticks: ticks.max(0) as u32,
            value,
            suspended: false,
        };
        Ok(CallbackReturn::Sequence(phodopus::BoxSequence::new(
            &ctx, seq,
        )))
    });
    ctx.set_global("mock_sleep", callback);
}

/// Step until the executor is done or parked on a host op. Returns the parked handle, if any.
fn step_until_parked(
    lua: &mut Lua,
    executor: &phodopus::StashedExecutor,
    fuel: &mut Fuel,
) -> Option<HostOpHandle> {
    loop {
        // `step` returning `true` means "no more progress can be made": that covers both
        // completion AND host-suspension (the parked sequence is not re-pollable). Check the mode
        // first; only loop on fuel exhaustion (`Normal` + out of fuel).
        let done = lua.enter(|ctx| ctx.fetch(executor).step(ctx, fuel).unwrap());
        let mode = lua.enter(|ctx| ctx.fetch(executor).mode());
        match mode {
            ExecutorMode::HostSuspended => {
                return lua.enter(|ctx| ctx.fetch(executor).pending_host_op(ctx));
            }
            ExecutorMode::Normal => {
                assert!(
                    !done,
                    "step reported done while still Normal (fuel exhausted instead?)"
                );
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

#[test]
fn mock_async_timer_resumes_with_correct_values() -> Result<(), ExternError> {
    let mut host = MockHost::default();
    let host_ptr: *mut MockHost = &mut host;
    let host_cell = Rc::new(Cell::new(host_ptr));

    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        install_mock_sleep(ctx, host_cell.clone());
        Ok(())
    })?;

    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            None,
            &br#"
                local v1 = mock_sleep(3, "wake")
                assert(v1 == "wake", "expected wake value, got " .. tostring(v1))
                return v1
            "#[..],
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;

    // Drive the trampoline: step until parked, advance the mock clock, resume when due.
    let mut fuel = Fuel::with(1_000_000);
    let mut resumed = false;
    for _ in 0..100 {
        match step_until_parked(&mut lua, &executor, &mut fuel) {
            None => break,
            Some(handle) => {
                let mode = lua.enter(|ctx| ctx.fetch(&executor).mode());
                assert_eq!(mode, ExecutorMode::HostSuspended);
                // Host advances the mock clock.
                let fired = host.tick();
                assert!(
                    !fired.is_empty() || host.timers.contains_key(&handle.raw()),
                    "parked op must be known to the host"
                );
                if let Some((_, value)) = fired.into_iter().find(|(h, _)| *h == handle) {
                    let mut result = HostOpResult::empty();
                    result.push(HostOpValue::Stashed(value));
                    lua.enter(|ctx| {
                        ctx.fetch(&executor)
                            .resume_host_op(ctx, handle, result)
                            .unwrap();
                    });
                    resumed = true;
                }
                fuel.refill(1_000_000, 1_000_000);
            }
        }
    }
    assert!(resumed, "timer should have fired and resumed the script");

    let v1 = lua.execute::<std::string::String>(&executor)?;
    assert_eq!(v1, "wake");
    Ok(())
}

#[test]
fn host_cancellation_surfaces_as_catchable_pcall_error() -> Result<(), ExternError> {
    let mut host = MockHost::default();
    let host_ptr: *mut MockHost = &mut host;
    let host_cell = Rc::new(Cell::new(host_ptr));

    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        install_mock_sleep(ctx, host_cell.clone());
        Ok(())
    })?;

    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            None,
            &br#"
                local ok, err = pcall(mock_sleep, 100, "never")
                assert(ok == false, "cancelled op must raise through pcall")
                return tostring(err)
            "#[..],
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;

    let mut fuel = Fuel::with(1_000_000);
    let handle = step_until_parked(&mut lua, &executor, &mut fuel)
        .expect("script should park on the mock timer");
    // Host policy cancels instead of resolving.
    host.cancel(handle);
    lua.enter(|ctx| {
        ctx.fetch(&executor)
            .cancel_host_op(ctx, handle, "operation timed out")
            .unwrap();
    });

    let msg = lua.execute::<std::string::String>(&executor)?;
    assert!(
        msg.contains("operation timed out"),
        "unexpected cancel message: {msg}"
    );
    Ok(())
}

#[test]
fn burst_of_100_suspended_coroutines_all_complete() -> Result<(), ExternError> {
    let mut host = MockHost::default();
    let host_ptr: *mut MockHost = &mut host;
    let host_cell = Rc::new(Cell::new(host_ptr));

    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        install_mock_sleep(ctx, host_cell.clone());
        Ok(())
    })?;

    // One executor per coroutine; each parks on its own mock timer with a distinct wake value.
    let mut executors = Vec::new();
    for i in 0..100i64 {
        let source = format!("return mock_sleep(1, {i})");
        let ex = lua.try_enter(|ctx| {
            let closure = Closure::load(ctx, None, source.as_bytes())?;
            Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
        })?;
        executors.push((i, ex));
    }

    // Step every executor until all are parked.
    let mut fuel = Fuel::with(1_000_000);
    let mut parked = Vec::new();
    for (i, ex) in &executors {
        let handle = step_until_parked(&mut lua, ex, &mut fuel)
            .unwrap_or_else(|| panic!("coroutine {i} should park"));
        parked.push((*i, ex.clone(), handle));
    }
    assert_eq!(parked.len(), 100);

    // Fire all timers at once (single mock tick), then resume every parked op non-blocking.
    let fired = host.tick();
    assert_eq!(fired.len(), 100, "all timers share one tick");
    let by_handle: HashMap<u64, StashedValue> =
        fired.into_iter().map(|(h, v)| (h.raw(), v)).collect();
    for (i, ex, handle) in &parked {
        let value = by_handle
            .get(&handle.raw())
            .unwrap_or_else(|| panic!("missing timer for coroutine {i}"))
            .clone();
        let mut result = HostOpResult::empty();
        result.push(HostOpValue::Stashed(value));
        lua.enter(|ctx| {
            ctx.fetch(ex).resume_host_op(ctx, *handle, result).unwrap();
        });
    }

    // Every coroutine completes with its own wake value.
    for (i, ex) in &executors {
        let got = lua.execute::<i64>(ex)?;
        assert_eq!(got, *i, "coroutine {i} resumed with wrong value");
    }
    Ok(())
}

#[test]
fn suspend_resume_preserves_fuel_and_quota_accounting() -> Result<(), ExternError> {
    let mut host = MockHost::default();
    let host_ptr: *mut MockHost = &mut host;
    let host_cell = Rc::new(Cell::new(host_ptr));

    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        install_mock_sleep(ctx, host_cell.clone());
        Ok(())
    })?;

    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            None,
            &br#"
                local v = mock_sleep(2, 7)
                assert(v == 7)
                local total = 0
                for i = 1, 100 do total = total + i end
                return total
            "#[..],
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;

    // Bounded fuel slice: step until parked, recording fuel consumed by the suspend transition.
    let mut fuel = Fuel::with(50_000);
    let handle = step_until_parked(&mut lua, &executor, &mut fuel).expect("script should park");
    let fuel_after_suspend = fuel.remaining();

    // While parked, further steps make no progress: `step` returns `true` immediately (the
    // parked sequence is not re-polled, so no `FUEL_PER_SEQ_STEP` burns) and the mode stays
    // `HostSuspended`. Fuel accounting is honest: only the suspending sequence step was charged;
    // parking itself charges nothing because the loop breaks before the per-iteration fuel/quota
    // charges at the bottom of the step body.
    for _ in 0..5 {
        lua.enter(|ctx| {
            let ex = ctx.fetch(&executor);
            let mut probe = Fuel::with(fuel_after_suspend);
            let done = ex.step(ctx, &mut probe).unwrap();
            assert!(done, "parked executor reports no further progress");
            assert_eq!(ex.mode(), ExecutorMode::HostSuspended);
            assert_eq!(
                probe.remaining(),
                fuel_after_suspend,
                "parked step must not burn fuel (sequence is not re-polled)"
            );
        });
    }

    // Resume through the quota chokepoint path and run to completion with a fresh slice.
    let fired = host.tick();
    assert!(fired.is_empty(), "timer needs 2 ticks");
    let fired = host.tick();
    assert_eq!(fired.len(), 1);
    let (_, value) = &fired[0];
    let mut result = HostOpResult::empty();
    result.push(HostOpValue::Stashed(value.clone()));
    lua.enter(|ctx| {
        ctx.fetch(&executor)
            .resume_host_op(ctx, handle, result)
            .unwrap();
    });
    // Registry entry is gone after resume.
    let pending = lua.enter(|ctx| {
        ctx.singleton::<Rootable![HostOpRegistry<'_>]>()
            .pending_count()
    });
    assert_eq!(pending, 0);

    let total = lua.execute::<i64>(&executor)?;
    assert_eq!(total, 5050);

    // Unknown-handle resume/cancel fails cleanly instead of corrupting state. Use a fresh parked
    // executor so the mode gate (HostSuspended) passes and the handle lookup is what fails.
    let executor2 = lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, &b"return mock_sleep(5, 0)"[..])?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    let mut fuel2 = Fuel::with(50_000);
    let _handle2 =
        step_until_parked(&mut lua, &executor2, &mut fuel2).expect("second script should park");
    let bogus = HostOpHandle::from_raw(u64::MAX);
    let err = lua.enter(|ctx| {
        ctx.fetch(&executor2)
            .resume_host_op(ctx, bogus, HostOpResult::empty())
            .unwrap_err()
    });
    assert!(
        format!("{err}").contains("unknown host operation"),
        "unexpected error: {err}"
    );
    let err = lua.enter(|ctx| {
        ctx.fetch(&executor2)
            .cancel_host_op(ctx, bogus, "nope")
            .unwrap_err()
    });
    assert!(
        format!("{err}").contains("unknown host operation"),
        "unexpected error: {err}"
    );
    Ok(())
}

#[test]
fn hostop_handle_finalizer_notifies_host_on_collect() -> Result<(), ExternError> {
    use std::sync::{Arc, Mutex};

    let dropped: Arc<Mutex<Vec<HostOpHandle>>> = Arc::new(Mutex::new(Vec::new()));
    let mut host = MockHost::default();
    let host_ptr: *mut MockHost = &mut host;
    let host_cell = Rc::new(Cell::new(host_ptr));

    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        install_mock_sleep(ctx, host_cell.clone());
        Ok(())
    })?;

    // Park one coroutine, then drop every Lua-side reference to its executor and collect.
    //
    // NOTE: the `StashedExecutor` must be dropped *outside* `Lua::enter` (i.e. not held across
    // calls): as long as the `DynamicRoot` lives, the thread stays rooted and collection cannot
    // reclaim it. `try_enter` scopes the stash, so returning only the handle out of the closure
    // releases the root before `gc_collect`.
    let handle = lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, &b"return mock_sleep(1000, 1)"[..])?;
        let ex = ctx.stash(Executor::start(ctx, closure.into(), ()));
        let e = ctx.fetch(&ex);
        let mut fuel = Fuel::with(1_000_000);
        let done = e.step(ctx, &mut fuel).unwrap();
        assert!(done);
        assert_eq!(e.mode(), ExecutorMode::HostSuspended);
        e.pending_host_op(ctx)
            .ok_or_else(|| phodopus::RuntimeError::new(TestError).into())
    })?;

    // The registry still tracks the parked op before collection.
    let pending = lua.enter(|ctx| {
        ctx.singleton::<Rootable![HostOpRegistry<'_>]>()
            .pending_count()
    });
    assert_eq!(pending, 1);

    // Collect: the dead thread's entry becomes abandonable; the host sweeps and drops the future.
    lua.gc_collect();
    let abandoned = lua.enter(|ctx| {
        ctx.singleton::<Rootable![HostOpRegistry<'_>]>()
            .abandoned_handles(&ctx)
    });
    assert!(
        abandoned.contains(&handle),
        "collected thread's op must be reported abandoned"
    );
    for h in &abandoned {
        host.cancel(*h);
        dropped.lock().unwrap().push(*h);
    }
    assert!(!host.timers.contains_key(&handle.raw()));
    assert_eq!(dropped.lock().unwrap().len(), 1);

    // A `HostOpGuard` also notifies synchronously on drop (unit-level finalizer contract).
    let notified = Arc::new(Mutex::new(Vec::new()));
    let notified2 = notified.clone();
    {
        let handle = HostOpHandle::new();
        let guard = phodopus::HostOpGuard::new(handle, move |h| {
            notified2.lock().unwrap().push(h);
        });
        assert_eq!(guard.handle(), handle);
        // Dropped here without disarm -> hook fires.
    }
    assert_eq!(notified.lock().unwrap().len(), 1);
    Ok(())
}

#[test]
fn suspend_goes_through_sequence_machinery_and_pending_still_works() -> Result<(), ExternError> {
    // Regression: `SequencePoll::Pending` semantics are unchanged (re-polled next step), while
    // `Suspend` parks (not re-polled until the host resolves).
    let mut lua = Lua::core();
    lua.try_enter(|ctx| {
        #[derive(Collect)]
        #[collect(require_static)]
        struct PendingThenReturn {
            polls: i32,
        }
        impl<'gc> Sequence<'gc> for PendingThenReturn {
            fn poll(
                self: Pin<&mut Self>,
                _ctx: Context<'gc>,
                _exec: Execution<'gc, '_>,
                mut stack: Stack<'gc, '_>,
            ) -> Result<SequencePoll<'gc>, Error<'gc>> {
                let this = self.get_mut();
                this.polls += 1;
                if this.polls < 3 {
                    Ok(SequencePoll::Pending)
                } else {
                    stack.replace(_ctx, Variadic(vec![this.polls]));
                    Ok(SequencePoll::Return)
                }
            }
        }
        let callback = Callback::from_fn(&ctx, |ctx, _, _| {
            Ok(CallbackReturn::Sequence(phodopus::BoxSequence::new(
                &ctx,
                PendingThenReturn { polls: 0 },
            )))
        });
        ctx.set_global("pending_cb", callback);
        Ok(())
    })?;

    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, &b"return pending_cb()"[..])?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;
    assert_eq!(lua.execute::<i32>(&executor)?, 3);
    Ok(())
}
