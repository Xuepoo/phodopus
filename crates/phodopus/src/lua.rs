use std::{ops, rc::Rc};

use gc_arena::{
    Arena, Collect, Mutation, Rootable,
    arena::{CollectionPhase, Root},
    metrics::Metrics,
};

use crate::{
    Error, ExternError, FromMultiValue, FromValue, Fuel, FuelExhausted, IntoValue, OutOfMemory,
    Registry, RuntimeError, Singleton, StashedExecutor, String, Table, TableError, TypeError,
    Value,
    finalizers::Finalizers,
    memory::MemoryLimit,
    stash::{Fetchable, Stashable},
    stdlib::{
        ModuleConfig, load_base, load_coroutine, load_debug, load_io, load_load_text, load_math,
        load_module, load_string, load_table, load_utf8,
    },
    string::InternedStringSet,
    thread::BadThreadMode,
};

/// A value representing the main "execution context" of a Lua state.
///
/// It provides access to the table of global variables, the registry, the string interner, and
/// other state that most every piece of running Lua code will need access to.
///
/// It is a cheap, copyable reference type that references internal state variables inside a [`Lua`]
/// instance.
///
/// As a convenience, it also contains the [`gc_arena::Mutation`] reference provided by `gc-arena`
/// when mutating a [`gc_arena::Arena`]. This allows code that uses piccolo to accept a single `ctx:
/// Context<'gc>` parameter, rather than having to accept both the piccolo `ctx` *and* the usual
/// `mc: &Mutation<'gc>` parameter.
///
/// To access the contained [`Mutation`] context, there is a `Deref` impl on `Context` that derefs
/// to `Mutation` that can be used like so:
///
/// ```
/// # use gc_arena::Gc;
/// # use phodopus::Lua;
/// # let mut lua = Lua::empty();
/// lua.enter(|ctx| {
///     // Create a new `Gc<'gc, i32>` pointer using the `&Mutation` held inside `ctx`
///     let p = Gc::new(&ctx, 13);
/// });
/// ```
#[derive(Copy, Clone)]
pub struct Context<'gc> {
    mutation: &'gc Mutation<'gc>,
    state: &'gc State<'gc>,
}

impl<'gc> Context<'gc> {
    /// Get a reference to [`Mutation`] (the `gc-arena` mutation handle) out of the `Context`
    /// object.
    ///
    /// This can also be done automatically with `Deref` coercion.
    pub fn mutation(self) -> &'gc Mutation<'gc> {
        self.mutation
    }

    /// The shared hard memory quota for this `Lua` instance.
    pub fn memory_limit(self) -> &'gc MemoryLimit {
        &self.state.memory_limit
    }

    /// Refuse `additional` more bytes unless the arena has room under the configured quota.
    ///
    /// Callers invoke this *before* growing a Lua-controlled collection (table array/map, large
    /// string buffer). It returns a typed [`OutOfMemory`] without allocating, so a refused request
    /// can never abort the process or leave a partially initialized value. See the scope note in
    /// `docs/specifications/sandbox-and-fuel.md` §4.2 for exactly which allocations are covered.
    pub fn check_memory(self, additional: usize) -> Result<(), OutOfMemory> {
        let current = self.metrics().total_allocation();
        self.memory_limit().check(current, additional)
    }

    /// Record the arena's current allocation into the quota and report whether it is over the
    /// ceiling. Does not refuse; used at GC boundaries where allocation already happened.
    pub fn observe_memory(self) -> bool {
        self.memory_limit()
            .observe(self.metrics().total_allocation())
    }

    pub fn globals(self) -> Table<'gc> {
        self.state.globals
    }

    pub fn registry(self) -> Registry<'gc> {
        self.state.registry
    }

    pub fn interned_strings(self) -> InternedStringSet<'gc> {
        self.state.strings
    }

    pub fn finalizers(self) -> Finalizers<'gc> {
        self.state.finalizers
    }

    pub fn string_metatable(self) -> Table<'gc> {
        self.state.string_metatable
    }

    // Calls `ctx.globals().get(key)`
    pub fn get_global<V: FromValue<'gc>>(self, key: &'static str) -> Result<V, TypeError> {
        self.state.globals.get(self, key)
    }

    // Calls `ctx.globals().get_value(key)`
    pub fn get_global_value(self, key: &'static str) -> Value<'gc> {
        self.state.globals.get_value(self, key)
    }

    // Calls `ctx.globals().set_field(key, value)`
    pub fn set_global<V: IntoValue<'gc>>(self, key: &'static str, value: V) -> Value<'gc> {
        self.state.globals.set_field(self, key, value)
    }

    /// Fallible variant of [`Context::set_global`] that returns a typed refusal instead of
    /// panicking when a hard memory quota is installed and the write would cross it.
    pub fn try_set_global<V: IntoValue<'gc>>(
        self,
        key: &'static str,
        value: V,
    ) -> Result<Value<'gc>, TableError> {
        self.state.globals.try_set_field(self, key, value)
    }

    /// Calls `ctx.registry().singleton::<S>(ctx)`.
    pub fn singleton<S>(self) -> &'gc Root<'gc, S>
    where
        S: for<'a> Rootable<'a> + 'static,
        Root<'gc, S>: Sized + Singleton<'gc> + Collect,
    {
        self.state.registry.singleton::<S>(self)
    }

    /// Calls `ctx.registry().stash(ctx, s)`.
    pub fn stash<S: Stashable<'gc>>(self, s: S) -> S::Stashed {
        self.state.registry.stash(&self, s)
    }

    /// Calls `ctx.registry().fetch(f)`.
    pub fn fetch<F: Fetchable>(self, f: &F) -> F::Fetched<'gc> {
        self.state.registry.fetch(f)
    }

    /// Calls `ctx.interned_strings().intern(&ctx, s)`.
    pub fn intern(self, s: &[u8]) -> String<'gc> {
        self.state.strings.intern(&self, s)
    }

    /// Calls `ctx.interned_strings().intern_static(&ctx, s)`.
    pub fn intern_static(self, s: &'static [u8]) -> String<'gc> {
        self.state.strings.intern_static(&self, s)
    }
}

impl<'gc> ops::Deref for Context<'gc> {
    type Target = Mutation<'gc>;

    fn deref(&self) -> &Self::Target {
        self.mutation
    }
}

/// A Lua execution environment.
///
/// This is the top-level `piccolo` type. In order to load and call any Lua code, the first step is
/// to create a `Lua` instance.
pub struct Lua {
    arena: Arena<Rootable![State<'_>]>,
    memory_limit: Rc<MemoryLimit>,
    fuel_limit: Option<i32>,
}

impl Default for Lua {
    fn default() -> Self {
        Lua::core()
    }
}

impl Lua {
    /// Start building a `Lua` instance with an explicit module configuration and explicit resource
    /// limits.
    ///
    /// This is the embedder hook for the sandboxed module system (virtual roots
    /// and compiled-in modules are injected here, never discovered from the
    /// host filesystem) and for the hard resource ceilings: see
    /// [`RuntimeBuilder::fuel_limit`] and [`RuntimeBuilder::memory_limit`].
    /// The default builder produces the same preload-only configuration as
    /// [`Lua::core`] with no limits.
    pub fn builder() -> LuaBuilder {
        LuaBuilder::new()
    }

    /// Create a new `Lua` instance with no parts of the stdlib loaded.
    pub fn empty() -> Self {
        let memory_limit = Rc::new(MemoryLimit::default());
        let limit = memory_limit.clone();
        Lua {
            arena: Arena::<Rootable![State<'_>]>::new(|mc| State::new(mc, limit)),
            memory_limit,
            fuel_limit: None,
        }
    }

    /// The shared hard memory quota for this instance.
    pub fn memory_limit(&self) -> &MemoryLimit {
        &self.memory_limit
    }

    /// The runtime's default execution Fuel budget, if one was configured.
    ///
    /// Set through [`RuntimeBuilder::fuel_limit`]; used by [`Lua::execute`] and
    /// [`Lua::execute_with_fuel`] when the caller does not pass an explicit budget.
    pub fn fuel_limit(&self) -> Option<i32> {
        self.fuel_limit
    }

    /// Install (or clear) the default execution Fuel budget.
    pub fn set_fuel_limit(&mut self, fuel: Option<i32>) {
        self.fuel_limit = fuel;
    }

    /// Install (or clear) the hard heap ceiling.
    ///
    /// This is the post-construction hook used by [`RuntimeBuilder::build`] to apply the quota
    /// *after* the trusted core standard library has been loaded, so the fixed stdlib footprint
    /// does not count against the script's budget. Setting a ceiling below the current tracked
    /// allocation is allowed and refuses the next checked allocation.
    pub fn set_memory_limit(&mut self, bytes: Option<usize>) {
        self.memory_limit.set_max_bytes(bytes);
    }

    /// Record the current tracked allocation without collecting.
    pub fn memory_used(&self) -> usize {
        self.memory_limit.current_bytes()
    }

    /// Whether the quota has refused an allocation since it was last cleared.
    pub fn memory_limit_exceeded(&self) -> bool {
        self.memory_limit.is_exceeded()
    }

    /// Attempt to reclaim memory with a full collection when the arena is at or above its quota,
    /// then report whether it remains over the ceiling.
    ///
    /// `gc-arena` forbids collection while the arena is mutably borrowed, so this is only callable
    /// between `Lua::enter` calls. It is invoked automatically between executor steps so that a
    /// script that crosses the ceiling triggers an immediate incremental collection before any
    /// further allocation is considered.
    fn collect_on_quota_pressure(&mut self) {
        let Some(max) = self.memory_limit.max_bytes() else {
            return;
        };
        if self.arena.metrics().total_allocation() >= max {
            self.collect_all_preserving_finalizers();
        }
    }

    /// Collect the whole arena, running finalizers, and refresh the observed quota.
    fn collect_all_preserving_finalizers(&mut self) {
        if self.arena.collection_phase() != CollectionPhase::Sweeping {
            if let Some(marked) = self.arena.mark_all() {
                marked.finalize(|fc, root| {
                    root.finalizers.prepare(fc);
                });
            }
            if let Some(marked) = self.arena.mark_all() {
                marked.finalize(|fc, root| {
                    root.finalizers.finalize(fc);
                });
            }
        }
        self.arena.collect_all();
        self.memory_limit
            .observe(self.arena.metrics().total_allocation());
    }

    /// Enforce the hard memory quota outside of arena mutation.
    ///
    /// If the tracked allocation is over the ceiling, a full collection is attempted; if the
    /// arena still exceeds the ceiling afterwards, a typed [`OutOfMemory`] is returned. This is
    /// the host-visible enforcement point that complements the per-allocation checks performed
    /// inside Lua table and string operations.
    pub fn enforce_memory_limit(&mut self) -> Result<(), OutOfMemory> {
        let Some(max) = self.memory_limit.max_bytes() else {
            return Ok(());
        };

        let mut current = self.arena.metrics().total_allocation();
        self.memory_limit.observe(current);
        if current <= max {
            return Ok(());
        }

        self.collect_all_preserving_finalizers();
        current = self.arena.metrics().total_allocation();
        self.memory_limit.observe(current);
        if current <= max {
            return Ok(());
        }

        Err(OutOfMemory {
            requested: current,
            limit: max,
            current,
        })
    }

    /// Create a new `Lua` instance with the core stdlib loaded.
    ///
    /// The module system is installed with an empty [`ModuleConfig`], so
    /// `require` exists but the VFS chain has no roots and resolves only
    /// preloaded modules.
    pub fn core() -> Self {
        let mut lua = Self::empty();
        lua.load_core();
        lua
    }

    /// Create a new `Lua` instance with all of the stdlib loaded.
    pub fn full() -> Self {
        let mut lua = Lua::core();
        lua.load_io();
        lua
    }

    /// Load the core parts of the stdlib that do not allow performing any I/O.
    ///
    /// Calls:
    ///   - `load_base`
    ///   - `load_coroutine`
    ///   - `load_math`
    ///   - `load_string`
    ///   - `load_table`
    ///   - `load_utf8`
    ///   - `load_debug`
    ///   - `load_module` (preload-only by default)
    pub fn load_core(&mut self) {
        self.load_core_with(&ModuleConfig::default());
    }

    /// Load the core parts of the stdlib with an explicit module configuration.
    pub fn load_core_with(&mut self, module_config: &ModuleConfig) {
        self.enter(|ctx| {
            load_base(ctx);
            load_coroutine(ctx);
            load_math(ctx);
            load_string(ctx);
            load_table(ctx);
            load_utf8(ctx);
            load_debug(ctx);
            load_module(ctx, module_config);
        })
    }

    /// Load the debug stdlib module.
    pub fn load_debug(&mut self) {
        self.enter(|ctx| {
            load_debug(ctx);
        })
    }

    /// Load the utf8 stdlib module.
    pub fn load_utf8(&mut self) {
        self.enter(|ctx| {
            load_utf8(ctx);
        })
    }

    /// Load the parts of the stdlib that allow I/O.
    pub fn load_io(&mut self) {
        self.enter(|ctx| {
            load_io(ctx);
        })
    }

    /// Load the parts of the stdlib that allow loading new code at runtime
    /// from text source code (not bytecode).
    pub fn load_load_text(&mut self) {
        self.enter(|ctx| {
            load_load_text(ctx);
        })
    }

    /// Size of all memory used by this Lua context.
    ///
    /// This is equivalent to `self.gc_metrics().total_allocation()`. This counts all `Gc` allocated
    /// memory and also all data Lua datastructures held inside `Gc`, as they are tracked as
    /// "external allocations" in `gc-arena`.
    pub fn total_memory(&self) -> usize {
        self.gc_metrics().total_allocation()
    }

    /// Finish the current collection cycle completely, calls `gc_arena::Arena::collect_all()`.
    pub fn gc_collect(&mut self) {
        if self.arena.collection_phase() != CollectionPhase::Sweeping {
            self.arena.mark_all().unwrap().finalize(|fc, root| {
                root.finalizers.prepare(fc);
            });
            self.arena.mark_all().unwrap().finalize(|fc, root| {
                root.finalizers.finalize(fc);
            });
        }

        self.arena.collect_all();
        assert!(self.arena.collection_phase() == CollectionPhase::Sleeping);
    }

    pub fn gc_metrics(&self) -> &Metrics {
        self.arena.metrics()
    }

    /// Enter the garbage collection arena and perform some operation.
    ///
    /// In order to interact with Lua or do any useful work with Lua values, you must do so from
    /// *within* the garbage collection arena. All values branded with the `'gc` branding lifetime
    /// must forever live *inside* the arena, and cannot escape it.
    ///
    /// Garbage collection takes place *in-between* calls to `Lua::enter`, no garbage will be
    /// collected concurrently with accessing the arena.
    ///
    /// Automatically triggers garbage collection before returning if the allocation debt is larger
    /// than a small constant.
    pub fn enter<F, T>(&mut self, f: F) -> T
    where
        F: for<'gc> FnOnce(Context<'gc>) -> T,
    {
        const COLLECTOR_GRANULARITY: f64 = 1024.0;

        let r = self.arena.mutate(move |mc, state| f(state.ctx(mc)));
        if self.arena.metrics().allocation_debt() > COLLECTOR_GRANULARITY {
            if self.arena.collection_phase() == CollectionPhase::Sweeping {
                self.arena.collect_debt();
            } else {
                if let Some(marked) = self.arena.mark_debt() {
                    marked.finalize(|fc, root| {
                        root.finalizers.prepare(fc);
                    });
                    self.arena.mark_all().unwrap().finalize(|fc, root| {
                        root.finalizers.finalize(fc);
                    });
                    // Immediately transition to `CollectionPhase::Sweeping`.
                    self.arena.mark_all().unwrap().start_sweeping();
                }
            }
        }
        r
    }

    /// A version of `Lua::enter` that expects failure and automatically converts [`Error`] into
    /// [`ExternError`], allowing the error type to escape the arena.
    pub fn try_enter<F, R>(&mut self, f: F) -> Result<R, ExternError>
    where
        F: for<'gc> FnOnce(Context<'gc>) -> Result<R, Error<'gc>>,
    {
        self.enter(move |ctx| f(ctx).map_err(Error::into_extern))
    }

    /// Run the given executor to completion.
    ///
    /// This will periodically exit the arena in order to collect garbage concurrently with running
    /// Lua code. If a hard memory quota is configured, the arena is checked between steps and a
    /// collection is triggered when it is at or above the ceiling.
    pub fn finish(&mut self, executor: &StashedExecutor) -> Result<(), BadThreadMode> {
        const FUEL_PER_GC: i32 = 4096;

        loop {
            let mut fuel = Fuel::with(FUEL_PER_GC);

            if self.enter(|ctx| ctx.fetch(executor).step(ctx, &mut fuel))? {
                break;
            }

            // Between steps the arena is not mutably borrowed, so a collection is legal. Do it
            // eagerly when the hard quota is at or above its ceiling so garbage is reclaimed
            // before the next allocation is considered.
            self.collect_on_quota_pressure();
        }

        Ok(())
    }

    /// Run the given executor to completion and then take return values from the returning thread.
    ///
    /// This is equivalent to calling `Lua::finish` on an executor and then calling
    /// `Executor::take_result` yourself. When a runtime Fuel budget is configured through
    /// [`RuntimeBuilder::fuel_limit`], it is enforced as a *total* budget for this call: a
    /// [`FuelExhausted`] error is returned if the script does not finish first.
    pub fn execute<R: for<'gc> FromMultiValue<'gc>>(
        &mut self,
        executor: &StashedExecutor,
    ) -> Result<R, ExternError> {
        if let Some(budget) = self.fuel_limit {
            self.execute_with_fuel(executor, Fuel::with(budget))
        } else {
            self.execute_with_fuel(executor, Fuel::with(i32::MAX))
        }
    }

    /// Run the given executor to completion with an explicit total Fuel budget.
    ///
    /// `fuel` is a single budget carried across every internal slice: it is refilled between
    /// garbage-collection boundaries but its remaining amount keeps decreasing, so the script
    /// stops when the total is consumed. This is the replenishment hook from the sandbox
    /// verification plan: to resume an interrupted executor, call this again with a refreshed
    /// budget, since the executor state is preserved across calls.
    ///
    /// A hard memory quota is enforced host-visibly before return values are taken, so a script
    /// that returned while the arena was over its ceiling yields a typed [`OutOfMemory`].
    pub fn execute_with_fuel<R: for<'gc> FromMultiValue<'gc>>(
        &mut self,
        executor: &StashedExecutor,
        fuel: Fuel,
    ) -> Result<R, ExternError> {
        const FUEL_PER_GC: i32 = 4096;

        let initial = fuel.remaining();
        let mut remaining = initial;
        loop {
            // Step with a bounded slice (carrying no more than the total budget still available),
            // then subtract what this slice consumed from the running total. This keeps the
            // configured budget a total rather than per-slice while still letting collection run
            // between steps.
            let slice = remaining.clamp(1, FUEL_PER_GC);
            let mut slice_fuel = Fuel::with(slice);
            let done = self
                .enter(|ctx| ctx.fetch(executor).step(ctx, &mut slice_fuel))
                .map_err(RuntimeError::new)?;
            remaining -= slice - slice_fuel.remaining();

            if done {
                break;
            }
            if remaining <= 0 {
                return Err(ExternError::from(RuntimeError::new(FuelExhausted {
                    limit: initial,
                })));
            }
            self.collect_on_quota_pressure();
        }

        self.enforce_memory_limit().map_err(RuntimeError::new)?;
        self.try_enter(|ctx| ctx.fetch(executor).take_result::<R>(ctx)?)
    }
}

/// A builder for a [`Lua`] instance with an explicit module configuration.
///
/// The builder exists so the embedder can register compiled-in modules and
/// capability roots *before* the runtime is created, keeping the module system
/// free of ambient host paths and filesystem discovery.
#[derive(Default, Clone)]
pub struct LuaBuilder {
    module_config: ModuleConfig,
    full: bool,
    fuel_limit: Option<i32>,
    memory_limit: Option<usize>,
}

/// The host-facing builder for a [`Lua`] runtime.
///
/// This is the name used by the sandbox specification's acceptance criteria
/// ([`RuntimeBuilder::fuel_limit`], [`RuntimeBuilder::memory_limit`]). It is the same builder
/// returned by [`Lua::builder`] and aliased to [`LuaBuilder`] so existing code keeps compiling.
pub type RuntimeBuilder = LuaBuilder;

impl LuaBuilder {
    fn new() -> Self {
        Self::default()
    }

    /// Register a compiled-in module source under a logical name.
    pub fn add_embedded_module(
        &mut self,
        name: impl Into<std::string::String>,
        source: impl Into<std::vec::Vec<u8>>,
    ) -> &mut Self {
        self.module_config.add_embedded_module(name, source);
        self
    }

    /// Register an empty capability root (virtual namespace) for the VFS.
    pub fn add_vfs_root(&mut self, namespace: impl Into<std::string::String>) -> &mut Self {
        self.module_config.add_vfs_root(namespace);
        self
    }

    /// Register a module source inside a capability root; the root is created
    /// implicitly when missing.
    pub fn add_vfs_module(
        &mut self,
        namespace: impl Into<std::string::String>,
        path: impl Into<std::string::String>,
        source: impl Into<std::vec::Vec<u8>>,
    ) -> &mut Self {
        self.module_config.add_vfs_module(namespace, path, source);
        self
    }

    /// Also load the I/O stdlib (equivalent to [`Lua`]'s `full` constructor).
    pub fn with_io(&mut self) -> &mut Self {
        self.full = true;
        self
    }

    /// Set the hard heap allocation ceiling for the runtime, in bytes.
    ///
    /// This is `RuntimeBuilder::memory_limit` from the sandbox specification. Once the runtime is
    /// built, any allocation that would push total tracked heap usage above `bytes` is refused with
    /// a clean [`OutOfMemory`] error; an incremental collection is attempted first so garbage is
    /// reclaimed before the runtime gives up. `0` is a valid hard ceiling (only an empty runtime
    /// can run). The quota applies to script-time allocation; the trusted core standard library
    /// loaded during construction is not charged against it.
    pub fn memory_limit(&mut self, bytes: usize) -> &mut Self {
        self.memory_limit = Some(bytes);
        self
    }

    /// Set the default execution Fuel budget for the runtime, in instruction units.
    ///
    /// This is `RuntimeBuilder::fuel_limit` from the sandbox specification. The budget is exposed
    /// through [`Lua::fuel_limit`] and applied by [`Lua::execute`]/[`Lua::execute_with_fuel`]
    /// unless the caller supplies an explicit budget. A non-positive budget is stored verbatim and
    /// causes any execution to interrupt immediately.
    pub fn fuel_limit(&mut self, fuel: i32) -> &mut Self {
        self.fuel_limit = Some(fuel);
        self
    }

    /// Consume the builder and produce a configured `Lua` instance.
    ///
    /// The core standard library is loaded first, then the hard memory quota is installed. This
    /// ordering is deliberate: the fixed, trusted runtime footprint is not charged against the
    /// script's quota, so a modest quota does not make the runtime unusable before any script runs.
    pub fn build(&self) -> Lua {
        let mut lua = Lua::empty();
        lua.load_core_with(&self.module_config);
        if self.full {
            lua.load_io();
        }
        lua.fuel_limit = self.fuel_limit;
        lua.set_memory_limit(self.memory_limit);
        // Seed the observed counter so `Lua::memory_used` is meaningful immediately.
        lua.enter(|ctx| ctx.observe_memory());
        lua
    }
}

#[derive(Collect)]
#[collect(no_drop)]
struct State<'gc> {
    globals: Table<'gc>,
    registry: Registry<'gc>,
    strings: InternedStringSet<'gc>,
    finalizers: Finalizers<'gc>,
    string_metatable: Table<'gc>,
    memory_limit: Rc<MemoryLimit>,
}

impl<'gc> State<'gc> {
    fn new(mc: &Mutation<'gc>, memory_limit: Rc<MemoryLimit>) -> State<'gc> {
        Self {
            globals: Table::new(mc),
            registry: Registry::new(mc),
            strings: InternedStringSet::new(mc),
            finalizers: Finalizers::new(mc),
            string_metatable: Table::new(mc),
            memory_limit,
        }
    }

    fn ctx(&'gc self, mutation: &'gc Mutation<'gc>) -> Context<'gc> {
        Context {
            mutation,
            state: self,
        }
    }
}
