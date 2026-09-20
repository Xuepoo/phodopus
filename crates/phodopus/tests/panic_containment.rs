//! Host-visible panic behavior of the Phodopus runtime.
//!
//! Per the accepted panic policy (see `docs/security/threat-model.md` and
//! `docs/security/unsafe-ledger.md`), Phodopus does **not** install a `catch_unwind` boundary.
//! A Rust panic raised inside the VM or a host callback unwinds across the runtime API and is
//! contained by the *host* at its own task/thread boundary. These tests pin that contract:
//!
//! 1. A Lua `error(...)` is a typed `Err`, never a panic.
//! 2. A panicking host callback propagates the unwind to the host's `catch_unwind`, which means the
//!    host is the component responsible for containment.

use std::panic::{AssertUnwindSafe, catch_unwind};

use phodopus::{Callback, CallbackReturn, Closure, Error, Executor, ExternError, Lua, Table};

/// A Lua-level error is reported as a typed error and must not unwind.
#[test]
fn lua_error_returns_typed_error_without_panicking() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    let executor = lua.try_enter(|ctx| {
        let closure = Closure::load(
            ctx,
            None,
            &br#"
                error("typed error")
            "#[..],
        )?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })?;

    let result = lua.execute::<()>(&executor);
    let error = result.expect_err("Lua error must be a typed Err, not a panic");

    assert!(
        format!("{error}").contains("typed error"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// A panicking host callback unwinds across the runtime API; the host contains it.
///
/// This is the executable statement of the host-responsibility panic policy: if Phodopus swallowed
/// panics, `catch_unwind` would observe `Ok`; the assertion below fails in that case.
#[test]
fn host_callback_panic_unwinds_to_host_boundary() {
    let mut lua = Lua::core();

    let executor = lua
        .try_enter(|ctx| {
            let boom = Callback::from_fn(&ctx, |_, _, _| -> Result<CallbackReturn, Error> {
                panic!("host callback panic")
            });
            let globals: Table = ctx.globals();
            globals.set_field(ctx, "boom", boom);

            let closure = Closure::load(
                ctx,
                None,
                &br#"
                    boom()
                "#[..],
            )?;
            Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
        })
        .expect("registering the panicking callback");

    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _ = lua.execute::<()>(&executor);
    }));

    let payload = caught.expect_err("panic must cross the runtime API, not be swallowed");
    let message = if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        String::from("<non-string panic payload>")
    };
    assert!(
        message.contains("host callback panic"),
        "unexpected panic payload: {message}"
    );
}
