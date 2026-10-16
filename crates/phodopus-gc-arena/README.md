# phodopus-gc-arena

Safe, incrementally garbage collected arenas.

## Lineage

This crate is a renamed registry fork of
[`gc-arena`](https://github.com/kyren/gc-arena) at the pinned revision
`5a7534b883b703f23cfb8c3cfdf033460aa77ea9` (upstream `gc-arena` version
`0.5.3`, committed 2024-07-23). The upstream registry release `0.5.3`
(2024-05-15) predates that pin and is API-incompatible, and the later
`0.6`/`0.7` releases remove or rework the APIs `phodopus` relies on.
The fork exists so `phodopus` can publish to crates.io without a git
dependency.

Upstream authorship (`kyren`, `moulins`, Aaron Hill) and the MIT license are
preserved. The original README is kept as `README-UPSTREAM.md` and the
original changelog as `CHANGELOG-UPSTREAM.md`. No source changes are
intended beyond the package rename and publish metadata.

## Compatibility

The fork keeps the pinned API surface intact for the `phodopus` Phase 3
quota accounting:

- `MetricsAlloc` in `allocator_api`
- `DynamicRootSet` and `DynamicRoot`
- `Collect`, `Mutation`, `Gc`, `GcWeak`, `Rootable`, `Static`
- `Collection`, `Finalization`, pacing via `metrics::Pacing` and
  `metrics::Metrics`, collection phases and debt handling

The library name remains `gc_arena`, so existing `use gc_arena::` paths
continue to work through a Cargo `package` rename. Version `0.5.3` under
the new name identifies the upstream content it was forked from.

## Maintenance

Upstream fixes should be cherry-picked from `kyren/gc-arena` onto this
fork. A move to a newer upstream line is a separate breaking task because
it rewrites quota accounting across the `phodopus` tree.
