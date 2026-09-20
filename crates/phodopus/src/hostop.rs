//! Typed host-async suspension bridge (Phase 4, `docs/specifications/async-trampoline.md`).
//!
//! The VM core stays synchronous and stackless: a [`Sequence`](crate::Sequence) that needs an
//! external future registers it with the host, receives an opaque [`HostOpHandle`], and returns
//! [`SequencePoll::Suspend`](crate::SequencePoll::Suspend). The executor parks the sequence in a
//! [`Frame::HostSuspended`](crate::thread::Thread) marker (native frames are not unwound) and
//! yields to the host trampoline with [`ExecutorMode::HostSuspended`](crate::ExecutorMode).
//!
//! GC isolation: only the opaque numeric handle crosses into host memory. The external future must
//! never hold a `Gc<'gc, T>` across a pending op; values transferred back on resume are either
//! primitives (`nil`, booleans, integers, numbers) or [`StashedValue`](crate::StashedValue)
//! handles rooted in a [`DynamicRootSet`](gc_arena::DynamicRootSet) (the sequence [`Locals`](crate::async_callback::Locals)
//! or the global registry), fetched back inside the arena on resume.
//!
//! Cancellation: when a Lua thread parked on a host op is garbage-collected or closed before the
//! host resolves the op, the [`HostOpGuard`] finalizer path notifies the host. Concretely, the
//! executor's [`HostOpRegistry`] singleton tracks every live `handle -> thread` entry; entries are
//! removed on resume/cancel, and [`HostOpRegistry::abandoned_handles`] lets the host sweep entries
//! whose thread died (the thread's weak entry no longer upgrades) and drop the paired future.
//! The deterministic test path is `Lua::gc_collect` followed by `abandoned_handles`.
//!
//! Complexity: handle allocation is `O(1)` time and space; registry operations are amortized `O(1)`
//! per op.

use std::{
    fmt,
    hash::Hash,
    sync::atomic::{AtomicU64, Ordering},
};

use gc_arena::{Collect, Gc, GcWeak, Mutation, lock::RefLock};
use hashbrown::HashMap;

use crate::{Context, Singleton, Thread, stash::Fetchable, thread::ThreadInner};

/// Opaque typed handle for a host-driven asynchronous operation.
///
/// Created by [`HostOpHandle::new`] when a Rust binding registers an external future with the host
/// bridge; the numeric value crosses into host memory while all GC values stay rooted in the arena
/// (see the module docs). `Copy + Hash + Eq` so hosts can use it as a map key for their own future
/// table. Never holds a `Gc` pointer.
///
/// Hosts MUST mint handles with [`HostOpHandle::new`] only. Do NOT fabricate handles from
/// [`HostOpHandle::from_raw`] counters: `from_raw` exists only to rebuild a handle already minted
/// by `new` (for example a raw value crossing an FFI boundary back into Rust). Handles that were
/// never minted by `new` can collide with a live parked operation, and the registry will reject
/// the second `register` under the same raw value (see [`HostOpRegistry::register`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostOpHandle(u64);

impl HostOpHandle {
    /// Allocate a fresh unique handle for one host operation.
    ///
    /// Uniqueness comes from a process-wide atomic counter, so handles are unique per process even
    /// across `Lua` instances. Wraps on `u64` exhaustion (practically unreachable).
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed).max(1))
    }

    /// The raw numeric value, for host-side map keys and logging.
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Rebuild a handle from a raw value previously produced by [`HostOpHandle::raw`].
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

impl Default for HostOpHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for HostOpHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "host-op#{}", self.0)
    }
}

// SAFETY: `HostOpHandle` is a plain `u64` with no GC pointers; tracing is a no-op
// (`needs_trace() == false`), so there is nothing for the collector to visit.
unsafe impl Collect for HostOpHandle {
    fn needs_trace() -> bool
    where
        Self: Sized,
    {
        false
    }
}

/// Error delivered when a pending host operation is cancelled.
///
/// Surface as a catchable Lua error: `cancel_host_op` converts this into an `Error::Runtime` that
/// unwinds through Lua `pcall` handlers normally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOpCancelled {
    /// The handle of the cancelled operation.
    pub handle: HostOpHandle,
    /// Host-provided reason (for example `"operation timed out"`).
    pub message: std::string::String,
}

impl fmt::Display for HostOpCancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "host operation {} cancelled: {}",
            self.handle, self.message
        )
    }
}

impl std::error::Error for HostOpCancelled {}

/// RAII guard tying a parked host operation to host-side cleanup.
///
/// Hold one inside the future-driving [`Sequence`](crate::Sequence) (or the binding that created
/// the op) so that dropping the sequence — for example when its Lua thread is collected or closed
/// — runs the registered `on_abandon` hook. The hook receives only the opaque [`HostOpHandle`]
/// (never GC pointers), so it can drop the paired external future from host memory.
///
/// The hook is `Fn(HostOpHandle) + Send + Sync + 'static`; the guard itself is `Collect` with
/// `require_static` (it holds no GC pointers), so it can live inside GC-traced sequences.
pub struct HostOpGuard {
    handle: HostOpHandle,
    on_abandon: Option<Box<dyn Fn(HostOpHandle) + Send + Sync + 'static>>,
}

impl HostOpGuard {
    /// Create a guard that calls `on_abandon(handle)` when dropped without [`HostOpGuard::disarm`].
    pub fn new(
        handle: HostOpHandle,
        on_abandon: impl Fn(HostOpHandle) + Send + Sync + 'static,
    ) -> Self {
        Self {
            handle,
            on_abandon: Some(Box::new(on_abandon)),
        }
    }

    /// The guarded handle.
    pub fn handle(&self) -> HostOpHandle {
        self.handle
    }

    /// Disarm the guard (for example after resume/cancel resolved the op); drop becomes a no-op.
    pub fn disarm(&mut self) {
        self.on_abandon = None;
    }
}

impl Drop for HostOpGuard {
    fn drop(&mut self) {
        if let Some(hook) = self.on_abandon.take() {
            hook(self.handle);
        }
    }
}

impl fmt::Debug for HostOpGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostOpGuard")
            .field("handle", &self.handle)
            .field("armed", &self.on_abandon.is_some())
            .finish()
    }
}

// SAFETY: no GC pointers inside (`HostOpHandle` is a u64, the hook is a `'static` Rust
// closure); tracing is a no-op (`needs_trace() == false`).
unsafe impl Collect for HostOpGuard {
    fn needs_trace() -> bool
    where
        Self: Sized,
    {
        false
    }
}

/// Tracks live `HostOpHandle -> thread` entries for one `Lua` instance.
///
/// Stored as a [`Singleton`] so the executor (which only has a [`Context`]) can register a parked
/// op without extra plumbing. Entries are keyed by the raw handle; the thread is held weakly so a
/// collected thread does not keep the entry (or itself) alive. [`Executor`](crate::Executor)
/// resume/cancel paths remove the entry they resolve; [`HostOpRegistry::abandoned_handles`] sweeps
/// entries whose thread died so the host can drop the paired future (the [`HostOpGuard`]
/// finalizer path).
#[derive(Debug, Collect)]
#[collect(no_drop)]
pub struct HostOpRegistry<'gc> {
    pending: Gc<'gc, RefLock<HostOpTable<'gc>>>,
}

#[derive(Debug, Default, Collect)]
#[collect(no_drop)]
struct HostOpTable<'gc> {
    entries: HashMap<u64, HostOpEntry<'gc>>,
}

#[derive(Debug, Clone, Copy, Collect)]
#[collect(no_drop)]
struct HostOpEntry<'gc> {
    thread: GcWeak<'gc, ThreadInner<'gc>>,
}

impl<'gc> Singleton<'gc> for HostOpRegistry<'gc> {
    fn create(ctx: Context<'gc>) -> Self {
        Self {
            pending: Gc::new(&ctx, RefLock::new(HostOpTable::default())),
        }
    }
}

impl<'gc> HostOpRegistry<'gc> {
    /// Record a parked op for `thread` under `handle`.
    ///
    /// Panics in debug builds if `handle` is already parked (a live duplicate registration would
    /// otherwise silently orphan the earlier thread's entry). Hosts MUST mint handles with
    /// [`HostOpHandle::new`], never [`HostOpHandle::from_raw`] counters, so distinct ops never
    /// share a raw value while both are parked.
    pub(crate) fn register(&self, ctx: &Context<'gc>, handle: HostOpHandle, thread: Thread<'gc>) {
        let entry = HostOpEntry {
            thread: Gc::downgrade(thread.into_inner()),
        };
        let previous = self
            .pending
            .borrow_mut(ctx)
            .entries
            .insert(handle.raw(), entry);
        debug_assert!(
            previous.is_none(),
            "duplicate HostOpHandle registration would orphan the parked thread"
        );
    }

    /// Remove the entry for `handle`, returning whether one existed.
    pub(crate) fn remove(&self, mc: &Mutation<'gc>, handle: HostOpHandle) -> bool {
        if let Ok(mut table) = self.pending.try_borrow_mut(mc) {
            table.entries.remove(&handle.raw()).is_some()
        } else {
            false
        }
    }

    /// Number of currently parked (unresolved) operations.
    pub fn pending_count(&self) -> usize {
        self.pending
            .try_borrow()
            .map(|t| t.entries.len())
            .unwrap_or(0)
    }

    /// Sweep entries whose thread died (collected/closed) and return their handles.
    ///
    /// The host calls this after `Lua::gc_collect` (or on a timer) and drops the paired external
    /// future for each returned handle. This is the [`HostOpGuard`] finalizer path made explicit
    /// and testable: GC never runs Rust `Drop` inside the arena, so notification is a host-driven
    /// sweep rather than an implicit destructor.
    pub fn abandoned_handles(&self, mc: &Mutation<'gc>) -> Vec<HostOpHandle> {
        let mut dead = Vec::new();
        if let Ok(mut table) = self.pending.try_borrow_mut(mc) {
            table.entries.retain(|raw, entry| {
                if entry.thread.upgrade(mc).is_some() {
                    true
                } else {
                    dead.push(HostOpHandle::from_raw(*raw));
                    false
                }
            });
        }
        dead
    }
}

/// Stashed host return values for [`Executor::resume_host_op`](crate::Executor::resume_host_op).
///
/// Primitives cross the boundary by value; GC values cross as [`StashedValue`] roots captured
/// before suspension (via sequence [`Locals`](crate::async_callback::Locals) or the global
/// registry) and are fetched back inside the arena on resume. This is the GC-isolation contract:
/// the external future itself never holds a `Gc<'gc, T>`.
#[derive(Debug, Clone, Default)]
pub struct HostOpResult {
    values: Vec<HostOpValue>,
}

/// One host return value: a primitive by value, or a GC value as a stashed root handle.
#[derive(Debug, Clone)]
pub enum HostOpValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    Stashed(crate::StashedValue),
}

impl HostOpResult {
    /// Empty result (resumes the sequence with no values).
    pub fn empty() -> Self {
        Self { values: Vec::new() }
    }

    /// Push one value.
    pub fn push(&mut self, value: HostOpValue) {
        self.values.push(value);
    }

    /// Build from an iterator of values.
    pub fn from_values(values: impl IntoIterator<Item = HostOpValue>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    /// Materialize the values inside the arena for delivery to the parked sequence's stack.
    pub(crate) fn fetch_into<'gc>(
        &self,
        roots: gc_arena::DynamicRootSet<'gc>,
        out: &mut Vec<crate::Value<'gc>>,
    ) {
        for v in &self.values {
            out.push(match v {
                HostOpValue::Nil => crate::Value::Nil,
                HostOpValue::Boolean(b) => crate::Value::Boolean(*b),
                HostOpValue::Integer(i) => crate::Value::Integer(*i),
                HostOpValue::Number(n) => crate::Value::Number(*n),
                HostOpValue::Stashed(s) => s.fetch(roots),
            });
        }
    }

    /// Number of values carried.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no values are carried.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl From<()> for HostOpResult {
    fn from(_: ()) -> Self {
        Self::empty()
    }
}

impl From<bool> for HostOpValue {
    fn from(v: bool) -> Self {
        HostOpValue::Boolean(v)
    }
}

impl From<i64> for HostOpValue {
    fn from(v: i64) -> Self {
        HostOpValue::Integer(v)
    }
}

impl From<i32> for HostOpValue {
    fn from(v: i32) -> Self {
        HostOpValue::Integer(v as i64)
    }
}

impl From<f64> for HostOpValue {
    fn from(v: f64) -> Self {
        HostOpValue::Number(v)
    }
}

impl From<crate::StashedValue> for HostOpValue {
    fn from(v: crate::StashedValue) -> Self {
        HostOpValue::Stashed(v)
    }
}
