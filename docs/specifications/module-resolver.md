---
title: Module Resolver Specification
description: Normative specification for sandboxed module loading, searcher chains, and VFS resolution
category: specifications
audience: developers
document_type: specification
design_status: accepted
implementation_status: complete
website_publish: true
sidebar_order: 23
---

# Module Resolver Specification

> Status: Design **accepted** | Implementation: **complete** (Phase 2). This document defines the sandboxed module loading architecture, searcher chains, and capability-constrained filesystem resolvers for `require` in Phodopus. The runtime implements the preload, embedded, VFS, and custom host searchers, the `package.loaded` cache with Lua 5.4 sentinel-`true` circular-dependency semantics, and an empty default `package.path` with no native loader.

---

## 1. Purpose and Scope

### In Scope

- Module lifecycle and caching in `package.loaded`.
- Pluggable searcher chains (`package.searchers`).
- Built-in and preloaded module registration (`package.preload`).
- Virtual filesystem (VFS) resolution with path normalization and path-traversal prevention.
- Detection and clean handling of circular module dependencies.

### Out of Scope

- Loading native C shared libraries (`.so`, `.dylib`, `.dll`).
- Direct access to arbitrary host OS paths outside registered virtual roots.

---

## 1.1 Normative Sources This Specification Must Not Weaken

- **Lua 5.4 Reference Manual, `require` and `package.searchers`**: the cache lookup, searcher
  iteration, and loader invocation order.
- **Lua 5.4 Reference Manual, `package.loaded` and `package.preload`**: module cache and
  preload registration semantics, including the sentinel-`true` circular-dependency protocol.
- **Bitty security policy and this repository's `docs/security/threat-model.md`**: no ambient
  host access, no native loader, and deny-by-default capability roots.

---

## 1.2 Terminology

- **Module name**: the dot-separated identifier passed to `require` (for example `foo.bar`).
- **Searcher**: a function in `package.searchers` that maps a module name to a loader function or
  a candidate description (section 2.2).
- **Capability root**: a host-injected virtual namespace inside which the VFS resolves modules.
- **Sentinel**: the boolean `true` written to `package.loaded[name]` before a loader runs, so a
  circular `require` sees `true` instead of re-evaluating.
- **Access violation**: the hard error raised when a module name fails pre-resolution validation.

---

## 2. Technical Specification

### 2.1 The `require` Execution Pipeline

When Lua code executes `local mod = require("foo.bar")`:

```text
require("foo.bar")
       |
       v
Check package.loaded["foo.bar"]  --- [Found] ---> Return cached module
       |
    [Not Found]
       v
Iterate package.searchers:
  1. Preload Searcher (package.preload)
  2. Embedded Module Searcher (compiled-in assets)
  3. Virtual Filesystem Searcher (sandboxed root)
  4. Custom Host Searcher (if registered)
       |
       +---> Loader found: compile / execute loader chunk
       |        |
       |        v
       |     Store result in package.loaded["foo.bar"]
       |     Return module
       |
       +---> No searcher found module
                |
                v
             Raise Lua error listing checked searcher candidates
```

### 2.2 Pluggable Searcher Chain

Searchers implement the standard Lua searcher contract:

```lua
function searcher(module_name)
    -- Returns loader_function, or error string describing searched location
end
```

In Rust, the searcher trait is exposed via:

```rust
pub trait ModuleSearcher<'gc>: Collect {
    fn search(
        &self,
        ctx: Context<'gc>,
        name: &str,
    ) -> Result<Option<Function<'gc>>, SearchError>;
}
```

`package.preload` carries the core built-in libraries already loaded into the
runtime (`string`, `table`, `math`, `coroutine`, `utf8`, `debug`, and `io` when
present). Each preloaded loader returns the corresponding global table, so
`require("string") == string` holds without any VFS access. A core library that
is not loaded (for example `io` under `Lua::core()`) is simply absent from
`package.preload` and fails to resolve.

### 2.3 Sandboxed Virtual Filesystem (VFS) Resolution

To support multi-file modules and plugins without exposing the host OS:

1. **Root Isolation**: The VFS searcher is bound to a virtual namespace (e.g. `plugin://`) or an explicitly designated root directory. Roots and module sources are injected by the host at build time; the runtime never discovers filesystem paths on its own.
2. **Path Normalization**: Dots in module names (`a.b.c`) map to virtual separators (`a/b/c.lua` or `a/b/c/init.lua`). For each root, both candidates are checked in that order.
3. **Traversal Prevention**: Relative components (`..`, `./`) and root escapes (`/`, `C:\`, `\\`) are rejected before resolution. Validation is a single pre-resolution check on the module name: the accepted alphabet is ASCII alphanumerics, `_`, `-`, and the `.` separator, and no segment may be empty, `.`, or `..`. Because `:` and `\` are outside the alphabet, drive letters and UNC prefixes are rejected for free. Any rejected name immediately triggers an access violation error before any table or filesystem lookup, so it can never reach a registered root.

#### 2.3.1 Host API

The host configures modules through `Lua::builder()` before creating the runtime, or through
`ModuleConfig` plus `Lua::load_core_with()`:

- `add_embedded_module(name, source)` registers a compiled-in module for the embedded searcher.
- `add_vfs_root(namespace)` declares an (initially empty) capability root.
- `add_vfs_module(namespace, path, source)` registers a module source inside a capability root,
  creating the root implicitly when missing.

Additional modules and searchers can be registered after construction inside the arena with
`register_preload`, `register_searcher` (a `ModuleSearcher` implementation),
`register_searcher_fn` (a plain closure), or `register_searcher_callback` (a raw callback for
searchers that need to drive an `Executor`). A custom searcher is appended after the built-in
preload, embedded, and VFS searchers; the resulting order is exactly the chain in section 2.1.
An empty `ModuleConfig` produces a preload-only runtime.

All resolution steps and loader compilation are charged against `Fuel` through the
proportional cost model in `stdlib/sandbox.rs`, so a hostile module name or an oversized source
cannot bypass preemption. The `require` pipeline is a resumable `Sequence`: each `poll` advances
exactly one searcher call or the final loader call.

### 2.4 Circular Dependency Handling

Circular `require` calls are handled according to standard Lua 5.4 semantics:

1. Before invoking the module loader, `package.loaded[modname]` is initialized to a sentinel boolean `true`.
2. If the module calls `require(modname)` during its own evaluation, the sentinel `true` is returned rather than re-evaluating the file.
3. Upon loader completion, `package.loaded[modname]` is updated with the final returned value.

---

## 3. Security Review

- **Zero Ambient Host Access**: Unlike standard PUC-Rio Lua's default `package.path = "./?.lua;/usr/local/share/lua/5.4/?.lua"`, Phodopus ships with an empty default search path.
- **No Native Code Injection**: `package.loadlib` and C searchers are completely absent, preventing unauthorized native code execution from Lua scripts.

---

## 4. Verification Plan

1. **VFS Traversal Test**: Attempt `require("../../../etc/passwd")`; assert resolution is rejected at validation with a security error.
2. **Circular Require Test**: Create module A requiring module B, which requires module A; verify execution terminates without recursion overflow.
3. **Preload Test**: Register a preloaded module closure in Rust; verify `require` successfully resolves and executes it without filesystem queries.
4. **Missing Module Test**: Assert the raised error lists the candidates checked by the searcher chain.

Verification evidence lives in `crates/phodopus/tests/require.rs` (Rust) and
`crates/phodopus/tests/scripts/require.lua` (default preload-only runtime).

---

## 5. Alternatives Considered

- **Host filesystem resolution by default**: rejected. It would reintroduce ambient OS access and
  make the sandbox depend on the embedding host's layout.
- **Mapping `require` to a Rust `Path` and canonicalizing on disk**: rejected. Canonicalization
  happens after the path reaches the OS, which is exactly when traversal must already have been
  refused.
- **C-style `package.path` templates per root**: deferred. Host-injected namespace/path pairs are
  sufficient for the plugin contract and keep the model auditable.

---

## 6. Affected Contracts

- `crates/phodopus/src/stdlib/module.rs`: the `require` pipeline, searcher chain, validation
  function, and host registration API.
- `crates/phodopus/src/lua.rs`: `Lua::builder`, `Lua::load_core_with`, and the default
  preload-only module configuration.
- `crates/phodopus/src/stdlib/mod.rs`: stdlib wiring.
- `crates/phodopus/tests/require.rs`, `crates/phodopus/tests/scripts/require.lua`: verification.

---

## 7. Open Points

- `require` argument coercion currently follows this runtime's implicit integer-to-string
  conversion; the returned error format for a non-string argument is not specified here.
- Custom host searchers that need to suspend on a host event must drive an `Executor` through
  `register_searcher_callback`; a dedicated async-searcher API is deferred to the async
  trampoline work.

---

## 8. Acceptance Criteria

- `just check` passes: formatting, typecheck, clippy, tests, links, unsafe ledger, actionlint,
  and markdownlint.
- Every verification-plan case passes.
- No module name can reach a registered root unless it passes pre-resolution validation.
- `package.path` defaults to the empty string and no `package.loadlib` or C searcher exists.
