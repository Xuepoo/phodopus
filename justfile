# phodopus justfile — quality gates and development tasks

default: check

check: fmt-check typecheck clippy test actionlint markdownlint

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

typecheck:
    cargo check --workspace --all-targets

clippy:
    cargo clippy --workspace --all-targets

test:
    cargo test --workspace --all-targets

msrv:
    cargo +1.85 check --workspace --all-targets

actionlint:
    actionlint .github/workflows/*.yml

markdownlint:
    markdownlint-cli2 "docs/**/*.md" "README.md" "AGENTS.md" "crates/**/*.md"

# Publish a redacted CarryCtx snapshot inside this repo (commander merge
# closeout only; never a git hook). `carryctx export --publication` redacts the
# bundle, stamps manifest.redacted, and commits one snapshot to the fixed ref
# `refs/heads/carryctx-snapshots`.
workflow-publish *args:
    bash scripts/workflow-publish.sh {{args}}

workflow-publish-dry *args:
    bash scripts/workflow-publish.sh --dry-run {{args}}

# Restore the local CarryCtx DB from the in-repo snapshot branch
# `refs/heads/carryctx-snapshots` (fresh-clone recipe).
workflow-import *args:
    bash scripts/workflow-import.sh {{args}}

workflow-import-dry *args:
    bash scripts/workflow-import.sh --dry-run {{args}}

clean:
    cargo clean
