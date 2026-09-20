//! Shared, deterministic Fuel cost model and checked output ceilings for the
//! variable-cost standard library callbacks.
//!
//! Every callback that can do an amount of work not fixed by the VM instruction
//! cost must charge that work against [`Fuel`](crate::Fuel) proportionally. The
//! constants here are the single source of truth for that model so that the
//! sandbox specification and the implementation cannot drift.
//!
//! The model is deliberately conservative and deterministic:
//!
//! * one unit of Fuel per byte of input scanned and per byte of output produced;
//! * a fixed charge per format directive expanded and per pattern search attempt;
//! * a checked byte ceiling on every growing output buffer that is independent
//!   of any future global heap quota.
//!
//! Resumable operations read the remaining Fuel to size one batch. A minimum
//! batch size guarantees forward progress even when the remaining Fuel is small,
//! exactly like the existing `table.pack` / `table.unpack` sequences.

use crate::Fuel;

/// Fuel charged for each output byte produced by a variable-cost operation.
pub(crate) const FUEL_PER_OUTPUT_BYTE: i32 = 1;

/// Fuel charged for each input byte scanned by a variable-cost operation.
pub(crate) const FUEL_PER_SCANNED_BYTE: i32 = 1;

/// Fuel charged for each `string.format` conversion directive expanded.
pub(crate) const FUEL_PER_FORMAT_DIRECTIVE: i32 = 4;

/// Fuel charged for advancing over one byte of a `string.pack` / `string.unpack`
/// / `string.packsize` format string.
pub(crate) const FUEL_PER_FORMAT_BYTE: i32 = 1;

/// Fuel charged per pattern-search attempt (candidate start position) tried by
/// the pattern engine when no match is found early.
pub(crate) const FUEL_PER_PATTERN_ATTEMPT: i32 = 16;

/// Minimum number of items a resumable operation processes in one `poll` even
/// when the remaining Fuel is below the ideal batch, so that the operation
/// always advances towards completion.
pub(crate) const MIN_WORK_BATCH: usize = 1024;

/// Checked allocation ceiling shared by `string.format`, `string.gsub`, and the
/// `utf8` assembly buffers. It is intentionally independent of the (future)
/// global heap quota so that a single callback cannot grow an unbounded Rust
/// buffer before the allocator-side limit is implemented.
pub(crate) const MAX_STDLIB_STRING_BYTES: usize = 16 * 1024 * 1024;

/// Adds a checked amount of output bytes to `total`, returning `None` on
/// arithmetic overflow or on exceeding [`MAX_STDLIB_STRING_BYTES`].
pub(crate) fn checked_output_growth(total: usize, additional: usize) -> Option<usize> {
    total
        .checked_add(additional)
        .filter(|&value| value <= MAX_STDLIB_STRING_BYTES)
}

/// Fuel cost of scanning `len` input bytes.
pub(crate) fn scanned_cost(len: usize) -> i32 {
    crate::fuel::count_fuel(FUEL_PER_SCANNED_BYTE, len)
}

/// Fuel cost of producing `len` output bytes.
pub(crate) fn output_cost(len: usize) -> i32 {
    crate::fuel::count_fuel(FUEL_PER_OUTPUT_BYTE, len)
}

/// Fuel cost of a bounded pattern search covering `attempts` candidate start
/// positions.
pub(crate) fn count_pattern_attempts(attempts: usize) -> i32 {
    crate::fuel::count_fuel(FUEL_PER_PATTERN_ATTEMPT, attempts)
}

/// Sizes one resumable work batch from the remaining Fuel. `per_item` is the
/// Fuel cost of a single item; the result is at least [`MIN_WORK_BATCH`] so the
/// caller always makes progress.
pub(crate) fn work_batch(fuel: &mut Fuel, per_item: i32) -> usize {
    let budget = fuel.remaining().max(0) as usize;
    let per_item = usize::try_from(per_item.max(1)).unwrap_or(1);
    (budget / per_item).max(MIN_WORK_BATCH)
}
