//! Sandboxed module system for Phodopus: `require`, `package.loaded`, the
//! pluggable searcher chain, and the capability-constrained virtual filesystem.
//!
//! This implementation follows `docs/specifications/module-resolver.md`:
//!
//! * the `require` pipeline checks `package.loaded` first, then walks
//!   `package.searchers` in order (preload -> embedded -> VFS -> custom host);
//! * circular dependencies are handled with the Lua 5.4 sentinel-`true`
//!   protocol, so a module that requires itself while loading sees `true`;
//! * the VFS is bound to host-injected virtual roots and performs path
//!   validation *before* any resolution, rejecting traversal, root escapes,
//!   drive letters, and UNC prefixes;
//! * `package.path` defaults to the empty string and no native loader
//!   (`package.loadlib`, C searchers) is ever exposed.
//!
//! All resolution and compilation work is charged against
//! [`Fuel`](crate::Fuel) through the proportional cost model in
//! [`super::sandbox`], so a hostile module name or an oversized source cannot
//! bypass preemption.

use std::{pin::Pin, string::String as StdString, vec::Vec as StdVec};

use gc_arena::Mutation;
// Re-exported so embedders implementing [`ModuleSearcher`] (which requires the
// GC [`Collect`] bound) have the trait and its derive in scope without taking
// a direct `gc-arena` dependency.
pub use gc_arena::Collect;

use crate::{
    BoxSequence, Callback, CallbackReturn, Closure, Context, Error, Execution, Function, Sequence,
    SequencePoll, Stack, String, Table, Value, error::LuaError, meta_ops,
    stdlib::sandbox::scanned_cost,
};

/// A hard, non-recoverable failure raised by a [`ModuleSearcher`].
///
/// Unlike a not-found result (which is reported as a candidate description and
/// lets `require` continue with the next searcher), a `SearchError` aborts the
/// whole `require` call. Security violations such as a rejected traversal path
/// must use this channel.
#[derive(Debug, Clone)]
pub struct SearchError(pub StdString);

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SearchError {}

/// A pluggable module searcher.
///
/// This mirrors the standard Lua searcher contract: given a module name, a
/// searcher either returns a loader [`Function`] (`Ok(Some(..))`), reports that
/// it has no candidate (`Ok(None)`), or raises a hard error (`Err(..)`).
///
/// Searchers are stored in `package.searchers` at the end of the built-in
/// chain and are therefore tried after the preload, embedded, and VFS
/// searchers. A searcher can only participate in the GC arena if it is
/// [`Collect`], so custom searchers are usually small, allocation-free types.
pub trait ModuleSearcher<'gc>: Collect {
    /// Attempt to resolve `name` into a loader function.
    fn search(&self, ctx: Context<'gc>, name: &str) -> Result<Option<Function<'gc>>, SearchError>;

    /// A human-readable description of the location this searcher checked.
    ///
    /// The default is intentionally generic; built-in searchers override it so
    /// the `require` failure message lists precise candidates.
    fn describe(&self, name: &str) -> StdString {
        format!("no module '<{name}>'")
    }
}

/// Plain-Rust module configuration accumulated by the embedder before the
/// module system is loaded.
///
/// Configuration deliberately contains no GC values: it is applied once inside
/// the arena by [`load_module`]. Host paths are never hardcoded; every root and
/// every module source is injected here.
#[derive(Debug, Default, Clone)]
pub struct ModuleConfig {
    embedded: StdVec<(StdString, StdVec<u8>)>,
    roots: StdVec<StdString>,
    vfs: StdVec<(StdString, StdString, StdVec<u8>)>,
}

impl ModuleConfig {
    /// Register a compiled-in module source under a logical name.
    pub fn add_embedded_module(
        &mut self,
        name: impl Into<StdString>,
        source: impl Into<StdVec<u8>>,
    ) -> &mut Self {
        self.embedded.push((name.into(), source.into()));
        self
    }

    /// Register an empty capability root (virtual namespace) for the VFS.
    ///
    /// A root that never receives files is still a valid search location, but
    /// the default configuration registers no roots, so the default runtime is
    /// preload-only.
    pub fn add_vfs_root(&mut self, namespace: impl Into<StdString>) -> &mut Self {
        let namespace = namespace.into();
        if !self.roots.contains(&namespace) {
            self.roots.push(namespace);
        }
        self
    }

    /// Register a module source inside a capability root.
    ///
    /// `path` is the virtual file path relative to the root (for example
    /// `foo/bar.lua`); the module name `foo.bar` normalizes to that path. The
    /// root is created implicitly when it does not already exist.
    pub fn add_vfs_module(
        &mut self,
        namespace: impl Into<StdString>,
        path: impl Into<StdString>,
        source: impl Into<StdVec<u8>>,
    ) -> &mut Self {
        let namespace = namespace.into();
        if !self.roots.contains(&namespace) {
            self.roots.push(namespace.clone());
        }
        self.vfs.push((namespace, path.into(), source.into()));
        self
    }

    /// True when the default (preload-only) VFS chain should be extended.
    pub fn has_vfs_roots(&self) -> bool {
        !self.roots.is_empty()
    }
}

/// Validate a Lua module name *before* any resolution is attempted.
///
/// Module names are dot-separated identifiers (`a.b.c`). The accepted alphabet
/// is ASCII alphanumerics, `_`, `-`, and the `.` separator. This rejects, in a
/// single place, every escape the specification calls out:
///
/// * relative components `..` and `.` (as well as empty segments such as
///   `a..b` or a trailing dot);
/// * leading `/` and any path separator (`/`, `\`);
/// * Windows drive letters (`C:`) and UNC prefixes (`\\`) because `:` and `\`
///   are outside the alphabet.
///
/// Because the check runs before any table or filesystem lookup, a rejected
/// name can never reach a registered root.
pub fn validate_module_path(name: &str) -> Result<(), StdString> {
    if name.is_empty() {
        return Err("module name is empty".into());
    }
    for ch in name.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.') {
            return Err(format!("invalid character '{ch}' in module path"));
        }
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err("empty module path segment".into());
    }
    for segment in name.split('.') {
        if segment.is_empty() {
            return Err("empty module path segment".into());
        }
        if segment == "." || segment == ".." {
            return Err("path traversal is not permitted".into());
        }
    }
    Ok(())
}

/// The core standard-library globals exposed as preloaded modules.
///
/// Each entry becomes a `package.preload[name]` loader that returns the
/// corresponding global table, matching the `require("string")`-style access
/// PUC-Rio Lua provides for its built-in libraries.
const CORE_BUILTINS: &[&str] = &["string", "table", "math", "coroutine", "utf8", "debug"];

fn access_violation<'gc>(ctx: Context<'gc>, module_name: &str, reason: &str) -> Error<'gc> {
    let message = format!("access violation: refusing module path '{module_name}': {reason}");
    LuaError::from(Value::String(ctx.intern(message.as_bytes()))).into()
}

/// Load the sandboxed module system (`require` and `package`) into `ctx`.
///
/// The searcher chain is assembled in specification order. Searchers for
/// embedded modules and the VFS are only installed when the host actually
/// registered content, so the default configuration is preload-only.
pub fn load_module<'gc>(ctx: Context<'gc>, config: &ModuleConfig) {
    let package = Table::new(&ctx);
    let loaded = Table::new(&ctx);
    let preload = Table::new(&ctx);
    let searchers = Table::new(&ctx);

    package.set_field(ctx, "loaded", loaded);
    package.set_field(ctx, "preload", preload);
    package.set_field(ctx, "searchers", searchers);
    // Zero ambient host access: the default search path is empty and there is
    // no `cpath` and no native loader.
    package.set_field(ctx, "path", "");
    ctx.set_global("package", package);

    // Core built-in modules resolve through the preload searcher, so
    // `require("string")` / `require("table")` / ... return the already-loaded
    // stdlib table without touching the VFS.
    for name in CORE_BUILTINS {
        let value = ctx.get_global_value(name);
        if value.is_nil() {
            continue;
        }
        let loader = Callback::from_fn_with(&ctx, value, |value, ctx, _, mut stack| {
            stack.replace(ctx, *value);
            Ok(CallbackReturn::Return)
        });
        preload
            .set(ctx, ctx.intern(name.as_bytes()), loader)
            .unwrap();
    }

    let mut index = 1i64;
    searchers
        .set(ctx, index, preload_searcher(&ctx, preload))
        .unwrap();
    index += 1;

    if !config.embedded.is_empty() {
        let embedded = Table::new(&ctx);
        for (name, source) in &config.embedded {
            embedded
                .set(
                    ctx,
                    ctx.intern(name.as_bytes()),
                    ctx.intern(source.as_slice()),
                )
                .unwrap();
        }
        searchers
            .set(ctx, index, embedded_searcher(&ctx, embedded, ctx.globals()))
            .unwrap();
        index += 1;
    }

    if config.has_vfs_roots() {
        let roots: StdVec<String<'gc>> = config
            .roots
            .iter()
            .map(|root| ctx.intern(root.as_bytes()))
            .collect();
        let files = Table::new(&ctx);
        for (namespace, path, source) in &config.vfs {
            let key = format!("{namespace}/{path}");
            files
                .set(
                    ctx,
                    ctx.intern(key.as_bytes()),
                    ctx.intern(source.as_slice()),
                )
                .unwrap();
        }
        searchers
            .set(ctx, index, vfs_searcher(&ctx, roots, files))
            .unwrap();
    }

    ctx.set_global("require", require_callback(&ctx, loaded, searchers));
}

/// Register a preloaded loader for `name` in `package.preload`.
///
/// A preloaded loader is a Lua [`Function`] and resolves without any filesystem
/// query. The module system must already be loaded.
pub fn register_preload<'gc>(ctx: Context<'gc>, name: &str, loader: Function<'gc>) {
    let preload: Table = package_field(ctx, "preload");
    preload
        .set(ctx, ctx.intern(name.as_bytes()), loader)
        .expect("preload key is a string");
}

/// Register a custom host searcher implemented with the [`ModuleSearcher`]
/// trait. It is appended after the built-in searchers.
pub fn register_searcher<'gc, S>(ctx: Context<'gc>, searcher: S)
where
    S: ModuleSearcher<'gc> + 'gc,
{
    register_searcher_callback(ctx, searcher_callback(&ctx, searcher));
}

/// Register a custom host searcher as a raw [`Callback`].
///
/// This is the escape hatch for searchers that need to drive an `Executor`
/// (for example searchers backed by an async source); the callback follows the
/// standard Lua searcher contract and must return either a loader function or
/// a candidate description string.
pub fn register_searcher_callback<'gc>(ctx: Context<'gc>, callback: Callback<'gc>) {
    let searchers: Table = package_field(ctx, "searchers");
    let next = searchers.length() + 1;
    searchers
        .set(ctx, next, callback)
        .expect("searcher index is an integer");
}

/// Register a custom host searcher from a plain closure, avoiding the need for
/// the caller to implement [`Collect`].
pub fn register_searcher_fn<'gc, F>(ctx: Context<'gc>, searcher: F)
where
    F: Fn(Context<'gc>, &str) -> Result<Option<Function<'gc>>, SearchError> + 'static,
{
    let callback = Callback::from_fn_with(&ctx, (), move |_, ctx, mut exec, mut stack| {
        let name: String = stack.consume(ctx)?;
        let text = format!("{}", name.display_lossy());
        exec.fuel().consume(scanned_cost(text.len()));
        match searcher(ctx, &text) {
            Ok(Some(function)) => {
                stack.replace(ctx, function);
            }
            Ok(None) => {
                stack.replace(ctx, format!("no module '<{text}>'"));
            }
            Err(SearchError(message)) => {
                return Err(access_violation(ctx, &text, &message));
            }
        }
        Ok(CallbackReturn::Return)
    });
    register_searcher_callback(ctx, callback);
}

fn package_field<'gc, V: crate::FromValue<'gc>>(ctx: Context<'gc>, field: &'static str) -> V {
    let globals = ctx.globals();
    let package: Table = globals
        .get(ctx, "package")
        .expect("module system must be loaded before package access");
    package
        .get(ctx, field)
        .unwrap_or_else(|_| panic!("package.{field} must be present"))
}

fn searcher_callback<'gc, S>(mc: &Mutation<'gc>, searcher: S) -> Callback<'gc>
where
    S: ModuleSearcher<'gc> + 'gc,
{
    Callback::from_fn_with(mc, searcher, |searcher, ctx, mut exec, mut stack| {
        let name: String = stack.consume(ctx)?;
        let text = format!("{}", name.display_lossy());
        exec.fuel().consume(scanned_cost(text.len()));
        let candidate = searcher.describe(&text);
        match searcher.search(ctx, &text) {
            Ok(Some(function)) => {
                stack.replace(ctx, function);
            }
            Ok(None) => {
                stack.replace(ctx, candidate);
            }
            Err(SearchError(message)) => {
                return Err(access_violation(ctx, &text, &message));
            }
        }
        Ok(CallbackReturn::Return)
    })
}

fn preload_searcher<'gc>(ctx: &Mutation<'gc>, preload: Table<'gc>) -> Callback<'gc> {
    Callback::from_fn_with(ctx, preload, |preload, ctx, _, mut stack| {
        let name: String = stack.consume(ctx)?;
        let loader = preload.get_value(ctx, name);
        if loader.is_nil() {
            stack.replace(
                ctx,
                format!("no field package.preload['{}']", name.display_lossy()),
            );
        } else {
            stack.replace(ctx, loader);
        }
        Ok(CallbackReturn::Return)
    })
}

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
struct EmbeddedSearcher<'gc> {
    embedded: Table<'gc>,
    env: Table<'gc>,
}

fn embedded_searcher<'gc>(
    ctx: &Mutation<'gc>,
    embedded: Table<'gc>,
    env: Table<'gc>,
) -> Callback<'gc> {
    Callback::from_fn_with(
        ctx,
        EmbeddedSearcher { embedded, env },
        |searcher, ctx, mut exec, mut stack| {
            let name: String = stack.consume(ctx)?;
            let text = format!("{}", name.display_lossy());
            exec.fuel().consume(scanned_cost(text.len()));
            let source = searcher
                .embedded
                .get_value(ctx, ctx.intern(text.as_bytes()));
            match source {
                Value::String(source) => {
                    exec.fuel().consume(scanned_cost(source.len() as usize));
                    let chunk_name = format!("@{text}");
                    let closure = Closure::load_with_env(
                        ctx,
                        Some(&chunk_name),
                        source.as_bytes(),
                        searcher.env,
                    )?;
                    stack.replace(ctx, closure);
                }
                _ => {
                    stack.replace(ctx, format!("no embedded module '{text}'"));
                }
            }
            Ok(CallbackReturn::Return)
        },
    )
}

#[derive(Collect)]
#[collect(no_drop)]
struct VfsSearcher<'gc> {
    roots: StdVec<String<'gc>>,
    files: Table<'gc>,
}

fn vfs_searcher<'gc>(
    ctx: &Mutation<'gc>,
    roots: StdVec<String<'gc>>,
    files: Table<'gc>,
) -> Callback<'gc> {
    Callback::from_fn_with(
        ctx,
        VfsSearcher { roots, files },
        |searcher, ctx, mut exec, mut stack| {
            let name: String = stack.consume(ctx)?;
            let text = format!("{}", name.display_lossy());
            exec.fuel().consume(scanned_cost(text.len()));
            // Validate before any table lookup: an escaping name never reaches a
            // registered root and never leaves the arena.
            if let Err(reason) = validate_module_path(&text) {
                return Err(access_violation(ctx, &text, &reason));
            }

            let normalized = text.replace('.', "/");
            let mut candidates = StdVec::new();
            for root in &searcher.roots {
                let root = format!("{}", root.display_lossy());
                let relative = format!("{root}/{normalized}");
                let lua_candidate = format!("{relative}.lua");
                let init_candidate = format!("{relative}/init.lua");
                for candidate in [&lua_candidate, &init_candidate] {
                    let value = searcher
                        .files
                        .get_value(ctx, ctx.intern(candidate.as_bytes()));
                    if let Value::String(source) = value {
                        exec.fuel().consume(scanned_cost(source.len() as usize));
                        let chunk_name = format!("@{candidate}");
                        let closure = Closure::load_with_env(
                            ctx,
                            Some(&chunk_name),
                            source.as_bytes(),
                            ctx.globals(),
                        )?;
                        stack.replace(ctx, closure);
                        return Ok(CallbackReturn::Return);
                    }
                }
                candidates.push(format!(
                    "no VFS module '{lua_candidate}' or '{init_candidate}'"
                ));
            }

            stack.replace(ctx, candidates.join("\n\t"));
            Ok(CallbackReturn::Return)
        },
    )
}

fn require_callback<'gc>(
    ctx: &Mutation<'gc>,
    loaded: Table<'gc>,
    searchers: Table<'gc>,
) -> Callback<'gc> {
    Callback::from_fn_with(
        ctx,
        (loaded, searchers),
        |&(loaded, searchers), ctx, mut exec, mut stack| {
            let name: String = stack.consume(ctx)?;
            let text = format!("{}", name.display_lossy());
            exec.fuel().consume(scanned_cost(text.len()));

            // Reject escaping names before the cache or any searcher is consulted.
            if let Err(reason) = validate_module_path(&text) {
                return Err(access_violation(ctx, &text, &reason));
            }

            let cached = loaded.get_value(ctx, name);
            if cached.to_bool() {
                stack.replace(ctx, cached);
                return Ok(CallbackReturn::Return);
            }

            Ok(CallbackReturn::Sequence(BoxSequence::new(
                &ctx,
                RequireSequence {
                    name,
                    loaded,
                    searchers,
                    index: 1,
                    phase: RequirePhase::Finding,
                    candidates: StdVec::new(),
                },
            )))
        },
    )
}

#[derive(Copy, Clone, Collect)]
#[collect(require_static)]
enum RequirePhase {
    Finding,
    Searched,
    Loading,
}

/// The resumable `require` pipeline.
///
/// Each `poll` advances exactly one step (one searcher call, or the loader
/// call), so the executor's per-step Fuel charge bounds resolution and the
/// loaded module's own execution is metered by the VM.
#[derive(Collect)]
#[collect(no_drop)]
struct RequireSequence<'gc> {
    name: String<'gc>,
    loaded: Table<'gc>,
    searchers: Table<'gc>,
    index: i64,
    phase: RequirePhase,
    candidates: StdVec<StdString>,
}

impl<'gc> Sequence<'gc> for RequireSequence<'gc> {
    fn poll(
        mut self: Pin<&mut Self>,
        ctx: Context<'gc>,
        mut exec: Execution<'gc, '_>,
        mut stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        // Resolution scans the module name against every searcher; charge the
        // name length proportionally on each step.
        exec.fuel().consume(scanned_cost(self.name.len() as usize));

        match self.phase {
            RequirePhase::Finding => {
                let searcher = self.searchers.get_value(ctx, self.index);
                self.index += 1;
                if searcher.is_nil() {
                    let message = self.not_found_message(ctx);
                    return Err(LuaError::from(message).into());
                }
                let function: Function = meta_ops::call(ctx, searcher)?;
                stack.replace(ctx, self.name);
                self.phase = RequirePhase::Searched;
                Ok(SequencePoll::Call {
                    bottom: 0,
                    function,
                })
            }
            RequirePhase::Searched => {
                match stack.get(0) {
                    Value::Function(function) => {
                        // Sentinel: mark the module as loading before invoking
                        // the loader so a circular require sees `true`. A hard quota refusal is
                        // propagated as a typed `OutOfMemory` rather than panicking.
                        self.loaded.set(ctx, self.name, true).map_err(Error::from)?;
                        self.phase = RequirePhase::Loading;
                        stack.replace(ctx, self.name);
                        Ok(SequencePoll::Call {
                            bottom: 0,
                            function,
                        })
                    }
                    Value::String(candidate) => {
                        self.candidates
                            .push(format!("{}", candidate.display_lossy()));
                        self.phase = RequirePhase::Finding;
                        Ok(SequencePoll::Pending)
                    }
                    other => {
                        let message = format!(
                            "module searcher for '{}' returned an unexpected {} value",
                            self.name.display_lossy(),
                            other.type_name()
                        );
                        Err(LuaError::from(Value::String(ctx.intern(message.as_bytes()))).into())
                    }
                }
            }
            RequirePhase::Loading => {
                let value = stack.get(0);
                let module = if value.is_nil() {
                    Value::Boolean(true)
                } else {
                    value
                };
                self.loaded
                    .set(ctx, self.name, module)
                    .map_err(Error::from)?;
                stack.replace(ctx, module);
                Ok(SequencePoll::Return)
            }
        }
    }

    fn error(
        self: Pin<&mut Self>,
        ctx: Context<'gc>,
        _exec: Execution<'gc, '_>,
        error: Error<'gc>,
        _stack: Stack<'gc, '_>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>> {
        // A loader that raised an error must not leave the sentinel behind, or
        // every later `require` would return `true` for a module that never
        // finished loading. Clearing the cache entry lets a caller retry after
        // fixing the module.
        if matches!(self.phase, RequirePhase::Loading) {
            // Best-effort cleanup: setting a `Nil` value removes a map entry and cannot allocate,
            // so this cannot fail; ignore a refusal defensively without masking the real error.
            let _ = self.loaded.set(ctx, self.name, Value::Nil);
        }
        Err(error)
    }
}

impl<'gc> RequireSequence<'gc> {
    fn not_found_message(&self, ctx: Context<'gc>) -> Value<'gc> {
        let mut message = format!("module '{}' not found:", self.name.display_lossy());
        if self.candidates.is_empty() {
            message.push_str("\n\tno searcher produced a candidate");
        } else {
            for candidate in &self.candidates {
                message.push_str("\n\t");
                message.push_str(candidate);
            }
        }
        Value::String(ctx.intern(message.as_bytes()))
    }
}
