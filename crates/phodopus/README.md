# phodopus

Core runtime crate for Phodopus, a pure-Rust stackless Lua 5.4 virtual machine
designed for sandboxing, cooperative scheduling, fuel-metered execution, and
seamless async integration.

## Features

- **Stackless architecture**: Execution state is maintained in GC-managed heap structures, enabling pausing, resuming, and serializing execution across Rust asynchronous boundaries.
- **Resilient sandboxing**: Memory allocation tracking via `gc-arena` and fuel-metered instruction limits prevent runaway CPU or memory usage.
- **Pure-Rust implementation**: Zero C dependencies, suitable for embedded and isolated environments.
- **Modular stdlib**: Opt-in standard library modules tailored for embedded environments and scripting hosts.
