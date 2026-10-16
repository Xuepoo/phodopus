# phodopus-gc-arena-derive

Procedural derive macros for
[`phodopus-gc-arena`](https://github.com/bitty-terminal/phodopus).

## Lineage

This crate is a renamed registry fork of
[`gc-arena-derive`](https://github.com/kyren/gc-arena) at the pinned revision
`5a7534b883b703f23cfb8c3cfdf033460aa77ea9` (upstream `gc-arena` version
`0.5.3`, committed 2024-07-23). The upstream registry release `0.5.3`
(2024-05-15) predates that pin and is API-incompatible, and the later
`0.6`/`0.7` releases are breaking rewrites. The fork preserves the pinned
source verbatim except for the package rename and publish metadata.

Upstream authorship (`kyren`, `moulins`, Aaron Hill) and the MIT license are
preserved. See `LICENSE-MIT`, `LICENSE-CC0`, and the upstream changelog
carried alongside this crate.

## Scope

Provides the `Collect` derive macro used by `phodopus-gc-arena`. The macro
emits `::gc_arena::` paths, which resolve through the `gc_arena` library
name kept by `phodopus-gc-arena` for compatibility. No API changes are
intended; the macro surface matches the pinned upstream exactly.
