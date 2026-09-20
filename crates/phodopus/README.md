# phodopus

Core runtime crate for Phodopus, a pure-Rust stackless Lua 5.4 virtual machine
designed for sandboxing, cooperative scheduling, fuel-metered execution, and
seamless async integration.

## Features

- **Stackless architecture**: Execution state is maintained in GC-managed heap structures, enabling pausing, resuming, and serializing execution across Rust asynchronous boundaries.
- **Sandboxing primitives**: `gc-arena` allocation accounting is exposed through `Lua::total_memory`, and instruction Fuel limits bound CPU during execution. Memory is currently **measured**, not **refused**: a hard allocator-enforced quota is planned for Phase 3 and is not yet implemented.
- **Pure-Rust implementation**: Zero C dependencies, suitable for embedded and isolated environments. `unsafe` is confined to specific VM and `gc-arena` primitives; see the [Unsafe Code Ledger](../../docs/security/unsafe-ledger.md).
- **Modular stdlib**: Opt-in standard library modules tailored for embedded environments and scripting hosts.
