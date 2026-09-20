# phodopus

Core runtime crate for Phodopus, a pure-Rust stackless Lua 5.4 virtual machine
designed for sandboxing, cooperative scheduling, fuel-metered execution, and
seamless async integration.

## Features

- **Stackless architecture**: Execution state is maintained in GC-managed heap structures, enabling pausing, resuming, and serializing execution across Rust asynchronous boundaries.
- **Sandboxing primitives**: `gc-arena` allocation accounting is exposed through `Lua::total_memory`, and instruction Fuel limits bound CPU during execution. `RuntimeBuilder::memory_limit` installs a hard heap quota, enforced by a single executor-loop chokepoint that checks the tracked allocation after every executor iteration — so any retained growth, including deep Lua recursion, is refused with a typed, `pcall`-catchable `OutOfMemory` bounded to one executor iteration of overshoot. Per-site pre-allocation checks refuse table constructors and growth, `..`/`table.concat` buffers, `Closure` allocations, and large `string.rep`/`string.format`/`string.gsub` buffers before they are allocated. Because collection is forbidden inside arena mutation, the executor only refuses; reclamation happens at the GC boundary between steps. See the scope and bound note in the [sandbox specification](../../docs/specifications/sandbox-and-fuel.md).
- **Pure-Rust implementation**: Zero C dependencies, suitable for embedded and isolated environments. `unsafe` is confined to specific VM and `gc-arena` primitives; see the [Unsafe Code Ledger](../../docs/security/unsafe-ledger.md).
- **Modular stdlib**: Opt-in standard library modules tailored for embedded environments and scripting hosts.
