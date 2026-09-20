//! Hard memory quota accounting for a single [`Lua`](crate::Lua) instance.
//!
//! A [`MemoryLimit`] is a shared byte ceiling plus an observable current byte count. It is
//! integrated with the `gc-arena` [`Metrics`] tracker: every allocation the arena performs (both
//! `Gc` boxes and external buffers routed through [`MetricsAlloc`](gc_arena::allocator_api::MetricsAlloc))
//! is reported by `Metrics::total_allocation()`, which is the authoritative "current" value the
//! quota is checked against.
//!
//! Enforcement is performed at allocation boundaries in the runtime. Before one of these growth
//! paths allocates, it calls [`Context::check_memory`](crate::Context::check_memory), which refuses
//! with a typed [`OutOfMemory`] when the requested bytes would push the arena above the configured
//! ceiling:
//!
//! * every Lua table constructor (`{...}`), charged for its initial array and map capacity *before*
//!   either part is allocated (`Table::try_new`);
//! * every table array/map growth, charged for the amortized growth request before the fallible
//!   reserve (`RawTable::try_reserve_array` / `try_reserve_map`);
//! * `..` / `table.concat` result buffers, charged for the projected size before `Vec::with_capacity`
//!   (`meta_ops::concat_many` / `concat_separated`);
//! * every closure created by the `Closure` opcode, charged for its `Gc`-boxed `ClosureInner` and
//!   upvalue vector before the box is allocated (`Closure::try_from_parts`), so a retained closure
//!   chain is refused at the ceiling rather than at the execution boundary;
//! * large standard-library string buffers such as `string.rep`, `string.format`, and `string.gsub`,
//!   charged for the projected output before it is appended (`stdlib::string`).
//!
//! The check runs *before* the growth is attempted, so the documented quota paths cannot trigger a
//! native abort, `handle_alloc_error`, or a partially-initialized value. It is not a literal
//! interception of every internal `Gc::new`: `gc-arena 0.5.3` does not route `Gc`-box allocation
//! through an application allocator, so allocations made *inside* a checked operation (for example
//! the upvalue `Gc` boxes read by the `Closure` opcode, or the arena's own interned-string and
//! bookkeeping nodes) are not each gated individually. They remain bounded because the operation
//! that requests them is charged first and the next checked allocation or the GC-boundary check
//! rejects once the tracked total reaches the ceiling. See the scope note in
//! `docs/specifications/sandbox-and-fuel.md` §4.2.
//!
//! The ceiling is optional: [`MemoryLimit::new(None)`](MemoryLimit::new) means "unbounded", which
//! preserves the historical measuring-only behavior.
//!
//! Complexity: every check is `O(1)` time and space.

use std::cell::Cell;

use gc_arena::Collect;
use thiserror::Error;

/// A clean, typed out-of-memory error.
///
/// This is the payload carried by [`RuntimeError`](crate::RuntimeError) when a memory quota is
/// exceeded. It is `'static + Send + Sync`, so it can be transported across the Lua boundary like
/// any other host error and downcast by the embedder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error(
    "out of memory: allocation request of {requested} bytes would exceed the {limit} byte quota \
     (current {current} bytes)"
)]
pub struct OutOfMemory {
    /// The total allocation the runtime attempted, in bytes (`current + additional`).
    pub requested: usize,
    /// The configured hard ceiling, in bytes.
    pub limit: usize,
    /// The arena's total tracked allocation when the request was made, in bytes.
    pub current: usize,
}

/// A hard byte ceiling on total heap allocations within one `gc-arena` instance.
///
/// `max_bytes` is the policy; `current_bytes` mirrors the arena's tracked total allocation. The
/// value is shared (via `Rc`) between the [`Lua`](crate::Lua) handle, the arena root state, and
/// every allocation-boundary check, so all of them observe one consistent counter.
///
/// The ceiling is a `Cell` so the owning [`Lua`](crate::Lua) can install it after the trusted core
/// standard library has been loaded; see [`LuaBuilder::memory_limit`](crate::RuntimeBuilder::memory_limit).
#[derive(Debug, Collect)]
#[collect(require_static)]
pub struct MemoryLimit {
    max_bytes: Cell<Option<usize>>,
    current_bytes: Cell<usize>,
    exceeded: Cell<bool>,
}

impl MemoryLimit {
    /// Create a new limit. `None` means unbounded (no refusal).
    pub fn new(max_bytes: Option<usize>) -> Self {
        Self {
            max_bytes: Cell::new(max_bytes),
            current_bytes: Cell::new(0),
            exceeded: Cell::new(false),
        }
    }

    /// The configured ceiling, if any.
    pub fn max_bytes(&self) -> Option<usize> {
        self.max_bytes.get()
    }

    /// Install (or clear) the ceiling.
    ///
    /// Setting a ceiling below the current tracked allocation is allowed; the next checked
    /// allocation or [`Lua::enforce_memory_limit`](crate::Lua::enforce_memory_limit) will refuse.
    pub fn set_max_bytes(&self, max_bytes: Option<usize>) {
        self.max_bytes.set(max_bytes);
    }

    /// The most recently observed total allocation, in bytes.
    ///
    /// This is updated by [`MemoryLimit::observe`] at every checked allocation boundary and by the
    /// [`Lua`](crate::Lua) GC boundary.
    pub fn current_bytes(&self) -> usize {
        self.current_bytes.get()
    }

    /// Whether a request has been refused since the flag was last cleared.
    pub fn is_exceeded(&self) -> bool {
        self.exceeded.get()
    }

    /// Clear the sticky "a request was refused" flag.
    pub fn clear_exceeded(&self) {
        self.exceeded.set(false);
    }

    /// Record the arena's current tracked allocation without refusing.
    ///
    /// Returns `true` if the observed total is above the ceiling.
    pub fn observe(&self, current: usize) -> bool {
        self.current_bytes.set(current);
        let over = self.max_bytes.get().is_some_and(|max| current > max);
        if over {
            self.exceeded.set(true);
        }
        over
    }

    /// Check whether `additional` more bytes may be allocated on top of `current`.
    ///
    /// Uses checked arithmetic so a hostile size cannot wrap and bypass the ceiling. On refusal the
    /// sticky exceeded flag is set and `current_bytes` is refreshed.
    pub fn check(&self, current: usize, additional: usize) -> Result<(), OutOfMemory> {
        self.current_bytes.set(current);
        let Some(max) = self.max_bytes.get() else {
            return Ok(());
        };

        let requested = current.checked_add(additional).ok_or_else(|| {
            self.exceeded.set(true);
            OutOfMemory {
                requested: usize::MAX,
                limit: max,
                current,
            }
        })?;

        if requested > max {
            self.exceeded.set(true);
            return Err(OutOfMemory {
                requested,
                limit: max,
                current,
            });
        }

        Ok(())
    }
}

impl Default for MemoryLimit {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_limit_never_refuses() {
        let limit = MemoryLimit::new(None);
        assert!(limit.check(usize::MAX / 2, usize::MAX / 2).is_ok());
        assert!(!limit.is_exceeded());
    }

    #[test]
    fn refuses_when_request_exceeds_ceiling() {
        let limit = MemoryLimit::new(Some(100));
        assert!(limit.check(40, 60).is_ok());
        let err = limit.check(40, 61).unwrap_err();
        assert_eq!(
            err,
            OutOfMemory {
                requested: 101,
                limit: 100,
                current: 40
            }
        );
        assert!(limit.is_exceeded());
    }

    #[test]
    fn checked_arithmetic_rejects_overflow() {
        let limit = MemoryLimit::new(Some(usize::MAX));
        let err = limit.check(usize::MAX, 1).unwrap_err();
        assert_eq!(err.requested, usize::MAX);
        assert!(limit.is_exceeded());
    }
}
