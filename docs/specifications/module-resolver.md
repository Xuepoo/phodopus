---
title: Module Resolver Specification
description: Normative specification for sandboxed module loading, searcher chains, and VFS resolution
category: specifications
audience: developers
document_type: specification
status: accepted
website_publish: true
sidebar_order: 23
---

# Module Resolver Specification

> Status: **accepted**. This document defines the sandboxed module loading architecture, searcher chains, and capability-constrained filesystem resolvers for `require` in Phodopus.

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

### 2.3 Sandboxed Virtual Filesystem (VFS) Resolution

To support multi-file modules and plugins without exposing the host OS:

1. **Root Isolation**: The VFS searcher is bound to a virtual namespace (e.g. `plugin://`) or an explicitly designated root directory.
2. **Path Normalization**: Dots in module names (`a.b.c`) map to virtual separators (`a/b/c.lua` or `a/b/c/init.lua`).
3. **Traversal Prevention**: Relative components (`..`, `./`) and root escapes (`/`, `C:\`, `\\`) are rejected before resolution. Any path attempting to escape the registered VFS boundary immediately triggers an access violation error.

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
