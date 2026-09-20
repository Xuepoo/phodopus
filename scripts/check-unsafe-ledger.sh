#!/usr/bin/env bash
# check-unsafe-ledger.sh — enforce the Phodopus unsafe ledger.
#
# Verifies three invariants against docs/security/unsafe-ledger.md:
#   1. Every Rust source file under crates/ has exactly the number of code `unsafe`
#      occurrences recorded in the ledger's machine manifest (no unreviewed
#      additions, no silently dropped sites).
#   2. Every `unsafe` occurrence is immediately preceded by a `SAFETY:` comment
#      justifying its invariants.
#   3. The stdlib and compiler trees contain zero `unsafe`.
#
# Full-line comments (`//`, `///`, `//!`) are ignored when counting, so prose may
# discuss unsafety without affecting the manifest.
#
# Usage: scripts/check-unsafe-ledger.sh
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LEDGER="${REPO_ROOT}/docs/security/unsafe-ledger.md"

if [[ ! -f "${LEDGER}" ]]; then
	echo "error: ledger not found: ${LEDGER}" >&2
	exit 1
fi

# Count code `unsafe` occurrences in one file, ignoring full-line comments.
count_unsafe() {
	awk '
        !/^[[:space:]]*\/\// && /(^|[^A-Za-z_])unsafe([^A-Za-z_]|$)/ { n++ }
        END { print n + 0 }
    ' "$1"
}

# Emit "line:<text>" for every code `unsafe` line in a file.
list_unsafe() {
	awk '
        !/^[[:space:]]*\/\// && /(^|[^A-Za-z_])unsafe([^A-Za-z_]|$)/ {
            print NR ":" $0
        }
    ' "$1"
}

fail=0

# --- 1. Manifest comparison ------------------------------------------------
declare -A expected=()
while IFS=' ' read -r path count; do
	[[ -z "${path}" ]] && continue
	expected["${path}"]="${count}"
done < <(
	awk '
        /<!-- unsafe-ledger:manifest:start -->/ { in_manifest = 1; next }
        /<!-- unsafe-ledger:manifest:end -->/   { in_manifest = 0; next }
        in_manifest && NF > 0 { print }
    ' "${LEDGER}"
)

if [[ "${#expected[@]}" -eq 0 ]]; then
	echo "error: ledger manifest is empty or markers are missing" >&2
	exit 1
fi

declare -A found=()
while IFS= read -r file; do
	rel="${file#"${REPO_ROOT}"/}"
	n="$(count_unsafe "${file}")"
	if [[ "${n}" -gt 0 ]]; then
		found["${rel}"]="${n}"
	fi
done < <(find "${REPO_ROOT}/crates" -name '*.rs' -type f | sort)

for path in "${!expected[@]}"; do
	if [[ ! -f "${REPO_ROOT}/${path}" ]]; then
		echo "FAIL: ledger records ${path} but the file does not exist" >&2
		fail=1
		continue
	fi
	actual="$(count_unsafe "${REPO_ROOT}/${path}")"
	if [[ "${actual}" != "${expected[${path}]}" ]]; then
		echo "FAIL: ${path}: ledger records ${expected[${path}]} unsafe site(s), found ${actual}" >&2
		fail=1
	fi
done

for path in "${!found[@]}"; do
	if [[ -z "${expected[${path}]:-}" ]]; then
		echo "FAIL: ${path}: ${found[${path}]} unsafe site(s) are not in the ledger" >&2
		fail=1
	fi
done

# --- 2. SAFETY comment coverage --------------------------------------------
while IFS= read -r file; do
	rel="${file#"${REPO_ROOT}"/}"
	while IFS= read -r entry; do
		[[ -z "${entry}" ]] && continue
		line="${entry%%:*}"
		start=$((line - 12))
		[[ "${start}" -lt 1 ]] && start=1
		if ! sed -n "${start},${line}p" "${file}" | grep -q 'SAFETY'; then
			echo "FAIL: ${rel}:${line}: unsafe site lacks a preceding SAFETY comment" >&2
			fail=1
		fi
	done < <(list_unsafe "${file}")
done < <(find "${REPO_ROOT}/crates" -name '*.rs' -type f | sort)

# --- 3. Forbidden trees ----------------------------------------------------
while IFS= read -r file; do
	rel="${file#"${REPO_ROOT}"/}"
	n="$(count_unsafe "${file}")"
	if [[ "${n}" -gt 0 ]]; then
		echo "FAIL: ${rel}: ${n} unsafe site(s) in a tree where unsafe is forbidden" >&2
		fail=1
	fi
done < <(
	find "${REPO_ROOT}/crates/phodopus/src/stdlib" \
		"${REPO_ROOT}/crates/phodopus/src/compiler" \
		-name '*.rs' -type f 2>/dev/null | sort
)

if [[ "${fail}" -ne 0 ]]; then
	echo "unsafe ledger check: FAILED" >&2
	exit 1
fi

total=0
for path in "${!expected[@]}"; do
	total=$((total + expected[${path}]))
done
echo "unsafe ledger check: OK (${total} recorded site(s) across ${#expected[@]} file(s))"
