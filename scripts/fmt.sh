#!/usr/bin/env bash
# The single source of truth for the formatting scope.
#
# The justfile, the pre-commit hook and CI all call this script, so the three can
# never drift apart — and none of them needs `just` to be installed.
#
# Vendored Zed crates (crates/vendor/**) are excluded on purpose: they are a
# pinned upstream snapshot carrying fmt drift that can never be made clean.
#
# Usage:
#   ./scripts/fmt.sh check [all|staged|branch [base-ref]]
#   ./scripts/fmt.sh fix   [all|staged|branch [base-ref]]
#
#   all     — every mezon-* crate (CI / just fmt-check default; NOT for task finish)
#   staged  — only staged .rs files (pre-commit hook; preferred for agents)
#   branch  — .rs files changed on this branch vs base-ref (fix/PR branches;
#             default base: origin/develop, else origin/main)
#
# Agents: prefer `fix staged` or `fix branch` or path-scoped `cargo fmt -- <paths>`.
# Never `cargo fmt --all` / `--workspace` (vendor is a workspace member).
set -euo pipefail

cd "$(dirname "$0")/.."

PKGS=(
    -p mezon-app
    -p mezon-ui
    -p mezon-store
    -p mezon-client
    -p mezon-native
    -p mezon-pdf
    -p mezon-proto
    -p mezon-i18n
    -p mezon-theme
    -p mezon-widgets
    -p mezon-canvas
    -p mezon-video
    -p mezon-updater
    -p mezon-audio
    -p mezon-voice
    -p mezon-stream
    -p mezon-record
    -p mezon-active-windows
    -p mezon-grammar
    -p mmn-client
)

RUSTFMT="${RUSTFMT:-rustfmt}"
EDITION=2024

is_scoped_rs() {
    local file=$1
    [[ "$file" == *.rs ]] || return 1
    [[ "$file" == crates/vendor/* ]] && return 1
    [[ "$file" == crates/mezon-*/* ]] && return 0
    [[ "$file" == crates/mmn-client/* ]] && return 0
    return 1
}

resolve_branch_base() {
    local base=${1:-}
    if [[ -n "$base" ]]; then
        echo "$base"
        return
    fi
    if git rev-parse --verify origin/develop >/dev/null 2>&1; then
        echo "origin/develop"
    elif git rev-parse --verify origin/main >/dev/null 2>&1; then
        echo "origin/main"
    elif git rev-parse --verify develop >/dev/null 2>&1; then
        echo "develop"
    elif git rev-parse --verify main >/dev/null 2>&1; then
        echo "main"
    else
        echo "HEAD~1"
    fi
}

collect_staged_paths() {
    git diff --cached --name-only --diff-filter=ACM | while IFS= read -r file; do
        [[ -n "$file" ]] || continue
        if is_scoped_rs "$file"; then
            echo "$file"
        fi
    done
}

collect_branch_paths() {
    local base
    base=$(resolve_branch_base "${1:-}")
    git diff --name-only --diff-filter=ACM "${base}...HEAD" | while IFS= read -r file; do
        [[ -n "$file" ]] || continue
        if is_scoped_rs "$file"; then
            echo "$file"
        fi
    done
}

run_rustfmt_paths() {
    local mode=$1
    shift

    local -a paths=()
    if (($# > 0)); then
        paths=("$@")
    fi

    if ((${#paths[@]} == 0)); then
        echo "No scoped Rust files to format." >&2
        exit 0
    fi

    if [[ "$mode" == check ]]; then
        "$RUSTFMT" --edition "$EDITION" --check "${paths[@]}"
    else
        "$RUSTFMT" --edition "$EDITION" "${paths[@]}"
    fi
}

run_cargo_fmt_all() {
    local mode=$1
    if [[ "$mode" == check ]]; then
        cargo fmt "${PKGS[@]}" -- --check
    else
        cargo fmt "${PKGS[@]}"
    fi
}

usage() {
    echo "usage: $0 [check|fix] [all|staged|branch [base-ref]]" >&2
    exit 2
}

ACTION=${1:-check}
SCOPE=${2:-all}
BASE=${3:-}

case "$ACTION" in
check | fix) ;;
*) usage ;;
esac

case "$SCOPE" in
all)
    run_cargo_fmt_all "$ACTION"
    ;;
staged)
    mapfile -t paths < <(collect_staged_paths)
    run_rustfmt_paths "$ACTION" "${paths[@]}"
    ;;
branch)
    mapfile -t paths < <(collect_branch_paths "$BASE")
    run_rustfmt_paths "$ACTION" "${paths[@]}"
    ;;
*) usage ;;
esac
