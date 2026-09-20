# phodopus justfile — quality gates, release verification, and development tasks
#
# Warning policy (CTX-0014 / review finding PHO-006)
# ---------------------------------------------------------------------------
# `just check` is warning-clean-or-fail: clippy runs under `-D warnings`.
# A frozen baseline of lints already firing in the inherited Piccolo fork source
# is explicitly allowed in `just clippy` so the gate is enforceable today without
# a mass source rewrite (source is owned by other tasks). The baseline is
# shrink-only: removing an entry is encouraged; adding one needs justification
# and review, because a new warning of a non-baselined lint (or any rustc-level
# warning) fails the gate.
# `just clippy-strict` is the target end state (empty baseline, `-D warnings`
# only); it is expected to fail until a follow-up clears the baseline sites.
# Measured baseline (2026-09-20, Rust 1.98.1, workspace --all-targets):
#   118 warnings in the `phodopus` lib, 15 in the `phodopus-util` lib;
#   141 unique warning sites across 38 clippy lints; no rustc-level warnings.
#   `clippy -- -D warnings` with no baseline: 120 compile errors.
#
# Scope of `just check`
# ---------------------------------------------------------------------------
# `just check` does NOT cover MSRV 1.85 or non-Linux platforms. MSRV is verified
# by the separate CI `msrv` job and locally by `just msrv`; Windows/macOS/Linux
# are the separate CI `Test (os)` matrix jobs. `just release-check` bundles the
# release-facing gates (MSRV, version consistency, package dry run, strict
# clippy) and is intentionally stricter than `just check`.

default: check

# Local/CI quality gate. Warning-clean-or-fail under the documented baseline.
check: fmt-check typecheck clippy test links unsafe-ledger actionlint markdownlint

# Release-facing gate. Stricter than `check`: adds MSRV, version consistency,
# Cargo package dry run, and the strict (empty-baseline) clippy policy. Expected
# to fail while the clippy baseline (tracked follow-up) and the gc-arena
# packaging gap (CTX-0016) remain; that failure is the honest release signal.
release-check: check msrv version-check package-check clippy-strict

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

typecheck:
    cargo check --workspace --all-targets

# Enforced warning policy: `-D warnings` plus the frozen, shrink-only baseline
# of lints already present in the inherited source. See the header for the
# measured baseline and rationale.
clippy:
    cargo clippy --workspace --all-targets -- \
        -A clippy::blocks_in_conditions \
        -A clippy::clone_on_copy \
        -A clippy::collapsible_match \
        -A clippy::derivable_impls \
        -A clippy::disallowed_names \
        -A clippy::explicit_auto_deref \
        -A clippy::get_first \
        -A clippy::if_same_then_else \
        -A clippy::implicit_saturating_sub \
        -A clippy::legacy_numeric_constants \
        -A clippy::len_without_is_empty \
        -A clippy::len_zero \
        -A clippy::let_and_return \
        -A clippy::manual_map \
        -A clippy::manual_range_contains \
        -A clippy::manual_strip \
        -A clippy::map_flatten \
        -A clippy::match_like_matches_macro \
        -A clippy::module_inception \
        -A clippy::multiple_bound_locations \
        -A clippy::needless_arbitrary_self_type \
        -A clippy::needless_bool \
        -A clippy::needless_borrow \
        -A clippy::needless_late_init \
        -A clippy::needless_lifetimes \
        -A clippy::needless_range_loop \
        -A clippy::neg_cmp_op_on_partial_ord \
        -A clippy::ptr_offset_with_cast \
        -A clippy::redundant_closure \
        -A clippy::redundant_pattern_matching \
        -A clippy::type_complexity \
        -A clippy::unnecessary_cast \
        -A clippy::unnecessary_lazy_evaluations \
        -A clippy::useless_conversion \
        -A clippy::useless_format \
        -A clippy::while_let_loop \
        -A clippy::while_let_on_iterator \
        -A clippy::write_with_newline \
        -D warnings

# Target end-state policy: no baseline allowlist. Documented, not part of
# `check`, and expected to fail until the baseline sites are fixed in source.
clippy-strict:
    cargo clippy --workspace --all-targets -- -D warnings

# Run tests. `tests/scripts-wishlist/` is a deliberately NON-REQUIRED corpus: the
# harness (`crates/phodopus/tests/scripts.rs`) reports its failures without
# failing the Rust test, and the `<close>` attribute case
# (`tests/scripts-wishlist/attributes.lua`) is a known, expected compile failure
# because close attributes are not implemented yet (see
# `tests/goldenscripts/close-unimpl.lua` and `close-multiple.lua`). Only
# `tests/scripts/` is required. This distinction is intentional and must not be
# read as full Lua 5.4 compatibility.
test:
    cargo test --workspace --all-targets

# MSRV verification. Kept out of `check` because it needs the separate 1.85
# toolchain; CI runs it as the standalone `msrv` job (see .github/workflows/ci.yml).
msrv:
    cargo +1.85 check --workspace --all-targets

# Release version consistency: every workspace member must declare the same
# version, and a tagged HEAD must match it. Tag/version identity repair is
# CTX-0016; this gate only asserts internal consistency.
version-check:
    #!/usr/bin/env bash
    set -euo pipefail
    versions="$(cargo metadata --no-deps --format-version 1 \
        | jq -r '.workspace_members as $m
            | .packages[]
            | select(.id as $i | $m | index($i))
            | .version')"
    unique="$(printf '%s\n' "${versions}" | sort -u)"
    if [[ "$(printf '%s\n' "${unique}" | wc -l)" -ne 1 ]]; then
        echo "FAIL: workspace members declare divergent versions:" >&2
        printf '  %s\n' ${unique} >&2
        exit 1
    fi
    version="${unique}"
    tag="$(git tag --points-at HEAD 2>/dev/null | grep -E '^v[0-9]' | head -n1 || true)"
    if [[ -n "${tag}" && "${tag#v}" != "${version}" ]]; then
        echo "FAIL: HEAD tag ${tag} does not match workspace version ${version}" >&2
        exit 1
    fi
    echo "version-check: OK (${version}${tag:+; tag ${tag}})"

# Cargo package dry run for every publishable workspace member. Currently fails
# on the git-only gc-arena dependency (no registry version requirement); the
# fix is owned by CTX-0016. Kept as a release gate so the defect cannot hide.
package-check:
    cargo package --workspace --allow-dirty --no-verify

# Repository-local Markdown link check: resolves every relative link target
# (fragment stripped, URLs skipped) against the tracked working tree. Catches
# renamed/removed docs before CI. Runs on the pinned toolchain's `git`.
links:
    #!/usr/bin/env bash
    set -euo pipefail
    status=0
    while IFS= read -r -d '' md; do
        dir="$(dirname "${md}")"
        while IFS= read -r target; do
            rel="${target%%#*}"
            case "${rel}" in
                '' | http://* | https://* | mailto:* | /*) continue ;;
            esac
            if [[ ! -e "${dir}/${rel}" && ! -e "${rel}" ]]; then
                printf 'broken link: %s -> %s\n' "${md}" "${target}" >&2
                status=1
            fi
        done < <(grep -oE '\]\([^)]+\)' "${md}" | sed -e 's/^](//' -e 's/)$//')
    done < <(git ls-files -z '*.md')
    if [[ "${status}" -eq 0 ]]; then
        echo "links: OK"
    fi
    exit "${status}"

# Enforce the unsafe ledger (CTX-0015). Owned by security; wiring is CTX-0014.
unsafe-ledger:
    bash scripts/check-unsafe-ledger.sh

actionlint:
    actionlint .github/workflows/*.yml

markdownlint:
    markdownlint-cli2 "docs/**/*.md" "README.md" "AGENTS.md" "crates/**/*.md"

# Publish a redacted CarryCtx snapshot inside this repo (commander merge
# closeout only; never a git hook). `carryctx export --publication` redacts the
# bundle, stamps manifest.redacted, and commits one snapshot to the fixed ref
# `refs/heads/carryctx-snapshots`.
workflow-publish *args:
    bash scripts/workflow-publish.sh {{ args }}

workflow-publish-dry *args:
    bash scripts/workflow-publish.sh --dry-run {{ args }}

# Restore the local CarryCtx DB from the in-repo snapshot branch
# `refs/heads/carryctx-snapshots` (fresh-clone recipe).
workflow-import *args:
    bash scripts/workflow-import.sh {{ args }}

workflow-import-dry *args:
    bash scripts/workflow-import.sh --dry-run {{ args }}

clean:
    cargo clean
