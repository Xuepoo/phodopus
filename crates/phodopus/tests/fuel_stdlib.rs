//! Low-limit Fuel accounting tests for the variable-cost stdlib callbacks.
//!
//! These tests deliberately use tiny Fuel budgets and modest inputs (never
//! megabytes of stress data). They prove two things:
//!
//! 1. a variable-cost operation is interrupted mid-flight, preserves its state,
//!    and resumes to the same result once Fuel is replenished; and
//! 2. the checked output ceilings stop a hostile operation before it can grow an
//!    unbounded buffer, returning a clean Lua error.

use phodopus::{Closure, Executor, ExternError, FromMultiValue, Lua, StashedExecutor};

/// Steps `executor` with a fresh `budget`-sized fuel slice until it finishes,
/// returning `(interruptions, result)`. `interruptions` counts how many
/// `Executor::step` calls exhausted their fuel, proving the operation was
/// preempted and resumed rather than running to completion in one step.
fn run_with_budget<R>(lua: &mut Lua, executor: &StashedExecutor, budget: i32) -> (usize, R)
where
    R: for<'gc> FromMultiValue<'gc>,
{
    let mut interruptions = 0usize;
    loop {
        let done = lua.enter(|ctx| {
            let mut fuel = phodopus::Fuel::with(budget);
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

    let result = lua
        .execute::<R>(executor)
        .expect("executor should finish cleanly");
    (interruptions, result)
}

fn start(lua: &mut Lua, source: &str) -> StashedExecutor {
    lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, None, source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    })
    .expect("script should compile")
}

/// Runs `executor` to completion in a single `step` with a huge budget and
/// returns the total Fuel consumed. A single step isolates the operation's own
/// charge from the per-step scheduling cost, so the measured value is a lower
/// bound on the proportional work charged by the callback under test.
fn run_to_completion_consumed(lua: &mut Lua, executor: &StashedExecutor) -> i32 {
    let budget = i32::MAX;
    let (done, consumed) = lua.enter(|ctx| {
        let mut fuel = phodopus::Fuel::with(budget);
        let done = ctx.fetch(executor).step(ctx, &mut fuel).unwrap();
        (done, budget - fuel.remaining())
    });
    assert!(done, "operation should complete within the huge budget");
    consumed
}

#[test]
fn format_is_interrupted_and_resumable() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // The format string is a long verbatim run, so the work is proportional to
    // its byte length and the sequence must yield many times under a 8-unit
    // budget instead of completing in one step.
    let source = r#"
        local body = string.rep("a", 64 * 1024)
        return string.format(body)
    "#;
    let executor = start(&mut lua, source);

    let (interruptions, result) = run_with_budget::<String>(&mut lua, &executor, 8);
    assert!(
        interruptions > 0,
        "format should have been interrupted at least once"
    );
    assert_eq!(result.len(), 64 * 1024);
    assert!(result.as_bytes().iter().all(|b| *b == b'a'));

    Ok(())
}

#[test]
fn format_output_ceiling_stops_hostile_growth() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // Each `%s` copies a 9 MiB string; two of them exceed the 16 MiB checked
    // ceiling and must fail cleanly rather than allocate ~18 MiB.
    let source = r#"
        local s = string.rep("x", 9 * 1024 * 1024)
        local ok, err = pcall(string.format, "%s%s", s, s)
        return tostring(ok) .. "|" .. tostring(err)
    "#;
    let executor = start(&mut lua, source);

    let (_, result) = run_with_budget::<String>(&mut lua, &executor, 1_000_000);
    assert!(
        result.starts_with("false|") && result.contains("resulting string too large"),
        "expected a checked ceiling error, got: {result}"
    );

    Ok(())
}

#[test]
fn gsub_is_interrupted_and_resumable() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    let source = r#"
        local s = string.rep("ab", 32 * 1024)
        return (string.gsub(s, "a", "c"))
    "#;
    let executor = start(&mut lua, source);

    let (interruptions, result) = run_with_budget::<String>(&mut lua, &executor, 8);
    assert!(
        interruptions > 0,
        "gsub should have been interrupted at least once"
    );
    let expected: String = "ab".repeat(32 * 1024).replace('a', "c");
    assert_eq!(result, expected);

    Ok(())
}

#[test]
fn gsub_output_ceiling_stops_hostile_growth() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // 1000 replacments of 20000 bytes each would produce ~20 MB, above the
    // 16 MiB checked ceiling.
    let source = r#"
        local s = string.rep("a", 1000)
        local r = string.rep("b", 20000)
        local ok, err = pcall(string.gsub, s, "a", r)
        return tostring(ok) .. "|" .. tostring(err)
    "#;
    let executor = start(&mut lua, source);

    let (_, result) = run_with_budget::<String>(&mut lua, &executor, 1_000_000);
    assert!(
        result.starts_with("false|") && result.contains("resulting string too large"),
        "expected a checked ceiling error, got: {result}"
    );

    Ok(())
}

#[test]
fn utf8_len_is_interrupted_and_resumable() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // 64 Ki code points, each 2 bytes, scanned one at a time.
    let source = r#"
        local s = string.rep("\u{00e9}", 64 * 1024)
        return utf8.len(s)
    "#;
    let executor = start(&mut lua, source);

    let (interruptions, result) = run_with_budget::<i64>(&mut lua, &executor, 8);
    assert!(
        interruptions > 0,
        "utf8.len should have been interrupted at least once"
    );
    assert_eq!(result, 64 * 1024);

    Ok(())
}

#[test]
fn utf8_len_reports_invalid_byte_after_resumption() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // A valid prefix followed by an invalid byte; the resumable scan must resume
    // and still report the 1-based offset of the first invalid byte.
    let source = r#"
        local s = string.rep("a", 4096) .. "\xff" .. string.rep("b", 4096)
        local n, err = utf8.len(s)
        return tostring(n) .. "|" .. tostring(err)
    "#;
    let executor = start(&mut lua, source);

    let (interruptions, result) = run_with_budget::<String>(&mut lua, &executor, 8);
    assert!(interruptions > 0);
    assert_eq!(result, "nil|4097");

    Ok(())
}

#[test]
fn pack_and_unpack_are_interrupted_and_resumable() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    let source = r#"
        local fmt = string.rep("I2", 8 * 1024)
        local vals = {}
        for i = 1, 8 * 1024 do vals[i] = i end
        local packed = string.pack(fmt, table.unpack(vals))
        local first, pos = string.unpack("I2", packed)
        return tostring(#packed) .. "|" .. tostring(first) .. "|" .. tostring(pos)
    "#;
    let fmt_len = "I2".len() * 8 * 1024;

    let executor = start(&mut lua, source);
    let (interruptions, result) = run_with_budget::<String>(&mut lua, &executor, 8);
    assert!(
        interruptions > 0,
        "pack/unpack should have been interrupted at least once (fmt {fmt_len})"
    );
    assert_eq!(result, format!("{}|1|3", 8 * 1024 * 2));

    Ok(())
}

#[test]
fn gsub_replacement_expansion_respects_ceiling() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // A single `a+` match of 1 MiB expanded once per `%0` directive 17 times
    // builds a ~17 MiB intermediate replacement buffer. The checked growth bound
    // must reject it before the buffer can exceed the 16 MiB ceiling, returning
    // the clean Lua error instead of allocating an unbounded buffer.
    let source = r#"
        local s = string.rep("a", 1024 * 1024)
        local r = string.rep("%0", 17)
        local ok, err = pcall(string.gsub, s, "a+", r)
        return tostring(ok) .. "|" .. tostring(err)
    "#;
    let executor = start(&mut lua, source);

    let (_, result) = run_with_budget::<String>(&mut lua, &executor, 1_000_000);
    assert!(
        result.starts_with("false|") && result.contains("resulting string too large"),
        "expected a checked ceiling error, got: {result}"
    );

    Ok(())
}

#[test]
fn gsub_charges_fuel_per_output_byte() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // `gsub` copies 200 input bytes to a 2000-byte replacement each, so the
    // output is 400_000 bytes. The pattern-attempt charge is only ~320_000, so
    // without per-output-byte Fuel charging the single-step total would stay
    // below the output size.
    let source = r#"
        local s = string.rep("a", 200)
        local r = string.rep("b", 2000)
        return (string.gsub(s, "a", r))
    "#;
    let executor = start(&mut lua, source);

    let consumed = run_to_completion_consumed(&mut lua, &executor);
    assert!(
        consumed >= 400_000,
        "gsub must charge ~1 Fuel per appended output byte; consumed {consumed}"
    );

    Ok(())
}

#[test]
fn tonumber_single_argument_charges_scanned_bytes() -> Result<(), ExternError> {
    let mut lua = Lua::core();

    // `string.rep("9", N)` charges N output bytes; the single-argument
    // `tonumber` scan of that N-byte string must add another N, so the total is
    // at least 2N. Without metering the scan the total would be only ~N.
    let n: i32 = 1024 * 1024;
    let source = format!(
        r#"
        local s = string.rep("9", {n})
        return tonumber(s)
        "#
    );
    let executor = start(&mut lua, &source);

    let consumed = run_to_completion_consumed(&mut lua, &executor);
    assert!(
        consumed >= 2 * n,
        "single-argument tonumber must charge the scanned bytes; consumed {consumed}"
    );

    Ok(())
}
