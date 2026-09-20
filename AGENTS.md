# phodopus agent guide

## Scope and authority

- This file governs only the independent `phodopus` Git repository under
  <https://github.com/bitty-terminal/phodopus>.
- The umbrella workspace directory is not a Git repository; other Bitty
  repositories own their own Git, CarryCtx, CI, releases, and agent guidance.
- Lineage & Attribution: `Phodopus` is the pure-Rust stackless Lua successor
  runtime forked from [`kyren/piccolo`](https://github.com/kyren/piccolo).
  All upstream commit history, copyright attributions, and dual MIT/CC0 licenses
  are strictly preserved. Upstream remote is tracked at
  `https://github.com/kyren/piccolo.git`.
- Canonical planning and design records live in this repository's `docs/`
  corpus, which bridges to `bitty-plugins-docs` and `bitty-terminal-docs` as
  integrations mature.
- Shared governance: decisions, security corpus, reviews, and project standards
  are defined in [`bitty-docs`](https://github.com/bitty-terminal/bitty-docs).

## Current phase

- Phase 0: Baseline fork initialized, repo identity, CarryCtx configuration, and
  workspace integration established; quality gates passing cleanly.
- Phase 1 (COMPLETE): Upstream PR absorption — `string.format` (#128), Lua
  pattern matching (#129), `utf8` (#110), `debug.traceback` (#121), plus stack
  frame safety coverage.
- Phase 1.5 (COMPLETE): Sandboxed text-only `load` (PR #91), custom `_ENV`
  binding, piecewise iterator chunks with a 16 MiB assembly ceiling, and global
  `_G`.
- Phase 2 (COMPLETE): Sandboxed module system (`require`) — pluggable searcher
  chain, preloaded core modules, an empty default `package.path`, and
  capability-constrained VFS roots.
- Phase 3 (IN PROGRESS): hard memory quotas are implemented (see the
  Sandboxing & Fuel boundary below); the broader Fuel-policy work plus
  host-agnostic async bridge (`HostOp::Pending`) and Bitty host ABI
  integration remain open.
  `docs/architecture/roadmap.md` is the single implementation truth for phase
  status.
- Do not introduce Bitty-specific abstractions or hardcoded async runtime
  assumptions into the VM core.

## Read before acting

1. Read this guide and the applicable files in `.carryctx/rules/`.
2. Adopt the assigned persona in `.carryctx/personas/`.
3. Read the task, team context, exact scopes, dependencies, and relevant
   governance contracts before modifying code or configurations.

## CarryCtx workflow

- CarryCtx is the durable execution record; it does not spawn agents.
- Bind a named agent and session to the task before work. Record progress,
  decisions, risks, blockers, handoffs, and checkpoints while work is active.
- Map GitHub Issue intent to a CarryCtx task; repository ownership to a team;
  ordering to dependencies; edits to exact scopes; active work to a session;
  and recovery points to checkpoints.
- Fresh clones restore the local CarryCtx DB from the in-repo snapshot branch
  with `just workflow-import` (validate-only: `just workflow-import-dry`). It
  fetches `refs/heads/carryctx-snapshots`, refuses to replace a non-empty local
  DB without `--force`, and prints provenance.
- Merge closeout publishes the in-repo snapshot via `just workflow-publish`
  (dry-run: `just workflow-publish-dry`), which commits a redacted export to
  `refs/heads/carryctx-snapshots` and pushes it to `origin`.

## Delivery lifecycle

- Normal lifecycle: GitHub Issue -> CarryCtx task -> team/dependencies/scopes ->
  named session -> isolated worktree and branch -> coherent commits -> pull request ->
  independent review + CI -> squash merge -> snapshot publication -> task completion -> Issue closure.
- After initialization, parallel implementation uses dedicated worktrees and
  branches. Branches follow `ctx-XXXX/<type>-<short-slug>` where `XXXX` is the
  owning CarryCtx task number, `<type>` is `feat|fix|chore|docs`, and the slug is
  kebab-case (e.g. `ctx-0001/feat-lua-patterns`); worktrees live at
  `.worktrees/ctx-XXXX-<type>-<short-slug>` with `/` mapped to `-`.
- One branch per task; commander housekeeping branches may use `cmd/<slug>`.
- Do not commit, push, merge, publish, or mutate remote state without explicit
  authorization from the user or owning task.

### GitHub hygiene (labels and milestones)

- Every GitHub Issue and PR carries labels (`feat`/`fix`/`docs`/`chore` +
  `P0`/`P1`/`P2` + `area:*`) and milestone (`v0.0.1`), created with
  `gh issue create --label ... --milestone ...` and kept in sync via `gh issue edit`
  or `gh pr edit`.
- Every task description and PR body includes:
  `Priority: ... | Area: ... | Labels: ... | Milestone: ... | Task: CTX-XXXX`.
- Merge method: squash merges only (`gh pr merge --squash --delete-branch`).
- Independent Review Gate: every PR must undergo rigorous review and verification before merge;
  never merge without green CI and confirmed verification against regressions.
- Documentation Synchronization: every code change must synchronously update affected
  canonical documents in `docs/` and crate READMEs.
- Proactive Enhancement: actively identify and implement opportunities to optimize memory
  consumption, execution efficiency, fuel budgeting precision, and API ergonomics.

### Quality gates before push (mandatory)

- Before pushing any branch: run repository justfile gates locally:
  - `just check` (runs `fmt-check`, `typecheck`, `clippy`, `test`)
  - `just fmt` (formats Rust code with `cargo fmt`)
  - `just actionlint` (validates all `.github/workflows/*.yml` files)
- Toolchain: pinned to Rust 1.98.1 (`rust-toolchain.toml`), edition 2024
  (`Cargo.toml` `[workspace.package]`), MSRV 1.85 (`Cargo.toml` `rust-version`,
  mirrored in `clippy.toml`).
- All tests must pass before proposing or merging any changes.

## Architectural boundaries

- **Strict Decoupling**: `Phodopus` is an independent, general-purpose,
  sandbox-first Lua runtime crate usable by any Rust application.
- **No Direct Tokio in VM Core**: Core VM execution remains synchronous and
  stackless. The planned asynchronous bridge will yield typed host suspension
  descriptors (`HostOp::Pending(handle)`), allowing external host schedulers
  (such as Tokio in Bitty) to drive futures and resume coroutines via a
  trampoline; this interface is Phase 4 target state and is not yet implemented.
- **Sandboxing & Fuel**: Execution is deterministic and preemptible by
  instruction Fuel today. A hard heap quota is implemented
  (`RuntimeBuilder::memory_limit`) with per-site pre-allocation checks plus a single
  executor-loop chokepoint: the tracked allocation is checked after every executor iteration,
  so any retained growth (including a deep Lua call chain, which has no per-site check) is
  refused with a typed, `pcall`-catchable `OutOfMemory` and the peak is bounded by the ceiling
  plus one quota-capped single-iteration allocation (measured ≤ 2× quota at 32 KiB and above;
  the production `Lua::execute` path steps with a bounded 4096-unit fuel slice, and a host
  driving `Executor::step` directly with an unbounded slice must supply its own quota
  discipline). This
  is complemented by per-site pre-allocation checks that refuse precisely
  _before_ a table constructor's initial storage, any later table array/map
  growth, a `table.pack`/`table.unpack` sequence batch (capped to the remaining quota),
  a `..`/`table.concat` result buffer, a `Closure` opcode's `Gc`-boxed
  closure, or a large `string.rep`/`string.format`/`string.gsub`/`string.pack`/`string.char`/
  `utf8.char` buffer is
  allocated. Collection cannot run inside arena mutation, so the executor only
  refuses; reclamation happens at the GC boundary between steps. The exact
  bound and scope are in `docs/specifications/sandbox-and-fuel.md` §4.2.
- **Native Lua Patterns**: String pattern matching implements authentic Lua
  patterns (via PR #129 adaptation) rather than generic Rust regex syntax.
- **No Hardcoded Values**: Never hardcode host/environment values: absolute
  paths, user home directories, hostnames, ports, credentials.
- **Documentation Language**: English-only for all code, comments, Markdown
  documentation, commit messages, and issue descriptions.
