#!/usr/bin/env bash
# sccache-worktree.sh
#
# rustc-wrapper for local + remote (cloud-build) compiles. Two jobs:
#   1. Keep per-worktree state OUT of the sccache Rust cache key: unsets
#      CARGO_TARGET_DIR (sccache hashes every CARGO_* env var into the Rust
#      key — src/compiler/rust.rs hash step 8 — and cloud-build build.sh
#      exports a per-worktree CARGO_TARGET_DIR on the VM). Without this,
#      every compile key diverges per .spur/worktrees/* and the shared
#      L0 disk + L1 S3 cache never hits.
#   2. Set SCCACHE_BASEDIRS for C/C++ compiles (via the VM's sccache-cc/-cxx
#      and any cc-rs invocation that inherits this env). NOTE: SCCACHE_BASEDIRS
#      does NOT affect Rust cache keys — sccache (0.14.0 → current) hashes the
#      rustc cwd raw and never normalizes it (upstream issue #2595, open);
#      strip_basedirs only rewrites C/C++ preprocessor output. Rust
#      cross-worktree sharing comes from job 1 (env hygiene) + stable registry
#      cwds; workspace crates additionally need canonical build dirs (see
#      docs/rca/2026-04-27 addendum).
#
# Backend selection (local builds default to the aws-my-aligned shared L1):
#   default             → two-level cache L0=local disk, L1=AWS S3
#                         (SCCACHE_MULTILEVEL_CHAIN=disk,s3; honored by
#                         sccache 0.15.0). Default bucket
#                         spurlab-591950085580-spur-sccache-apse5 in ap-southeast-5,
#                         matching the aws-my primary builder.
#   SPUR_SCCACHE_S3=0  → disable the default S3 backend.
# S3 credentials use the standard AWS CLI credential chain.
#
# Why (historical, corrected — see the RCA addendum): the original wrapper
# relied on SCCACHE_BASEDIRS to equalize worktree paths in the Rust cache key.
# That mechanism does not exist for Rust in sccache 0.14.0 → current (BASEDIRS
# only rewrites C/C++ preprocessor output); the measured cross-worktree hits
# were registry dependencies, whose rustc cwd is the shared
# $CARGO_HOME/registry/src path. What actually equalizes Rust keys today:
# unsetting CARGO_TARGET_DIR (above) + the shared registry home on the VM.
#
# See: docs/rca/2026-04-27-sccache-worktree-cache-miss.md (2026-09-23 addendum)
set -euo pipefail

# Resolve the git toplevel of the current working directory. In a git worktree
# this is the worktree root; in the main repo it is the repo root.
GIT_ROOT=""
if command -v git >/dev/null 2>&1; then
    GIT_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
fi

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
if [[ "$SCRIPT_DIR" == "$SCRIPT_PATH" ]]; then
    SCRIPT_DIR="."
fi

# Always include the main SPUR repo root as a fallback so registry caches and
# shared artifacts normalize consistently.
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd -P)
SPUR_ROOT="${SPUR_ROOT:-$REPO_ROOT}"

# Retire inherited Google cache settings so they cannot override AWS selection.
unset SCCACHE_GCS_BUCKET SCCACHE_GCS_RW_MODE SCCACHE_GCS_KEY_PATH
case "${SCCACHE_MULTILEVEL_CHAIN:-}" in
    *gcs*) unset SCCACHE_MULTILEVEL_CHAIN ;;
esac

use_spur_s3_sccache() {
    case "${SPUR_SCCACHE_S3-__spur_unset__}" in
        0|false|False|FALSE|no|No|NO|off|Off|OFF)
            return 1 ;;
        __spur_unset__)
            return 0 ;;
        *)
            return 0 ;;
    esac
}

# Two-level cache: L0=local disk (fast), L1=AWS S3 (shared, durable). sccache
# 0.15+ implements this natively via SCCACHE_MULTILEVEL_CHAIN; on an L1 hit it
# backfills L0. Returns 0 when activated, 1 when S3 is disabled.
enable_spur_s3_cache() {
    use_spur_s3_sccache || return 1

    # L1: AWS S3. SCCACHE_REGION MUST match the bucket's region or S3 rejects
    # the request. Bucket suffix apse5 == ap-southeast-5 (Malaysia), matching
    # the aws-my primary builder's cloud-build config so local and remote
    # builds share the same S3 L1 cache.
    export SCCACHE_BUCKET="${SCCACHE_BUCKET:-spurlab-591950085580-spur-sccache-apse5}"
    export SCCACHE_REGION="${SCCACHE_REGION:-ap-southeast-5}"
    export AWS_REGION="${AWS_REGION:-$SCCACHE_REGION}"
    # Keep the multilevel-configured server RESIDENT: sccache's default 10-min
    # idle timeout lets the S3-configured server die between builds, after which
    # unconfigured client auto-spawns come up disk-only and S3 stops working
    # until the next explicit restart (observed 2026-09-23). 0 = never idle.
    export SCCACHE_IDLE_TIMEOUT="${SCCACHE_IDLE_TIMEOUT:-0}"
    # Credentials resolve through the standard AWS chain (env vars, then the
    # default profile in ~/.aws/credentials, then IMDS).

    # L0: local disk. The `disk` level REQUIRES SCCACHE_DIR to be set explicitly
    # ("Disk cache specified in levels but not configured"), so default it to the
    # platform cache dir sccache already uses to reuse any existing local cache.
    if [[ -z "${SCCACHE_DIR:-}" ]]; then
        case "$(uname -s 2>/dev/null || echo "")" in
            Darwin) export SCCACHE_DIR="$HOME/Library/Caches/Mozilla.sccache" ;;
            *)      export SCCACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/sccache" ;;
        esac
    fi
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-10G}"

    export SCCACHE_MULTILEVEL_CHAIN="${SCCACHE_MULTILEVEL_CHAIN:-disk,s3}"
    # l0: only an L0 write failure is fatal; tolerate transient S3 write errors
    # so a flaky network never reds a build.
    export SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY="${SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY:-l0}"
    return 0
}


if [[ -n "$GIT_ROOT" && "$GIT_ROOT" != "$SPUR_ROOT" ]]; then
    # Worktree: strip to the worktree root first (longest-prefix wins).
    export SCCACHE_BASEDIRS="${GIT_ROOT}:${SPUR_ROOT}"
else
    # Main repo or not inside git: just the repo root.
    export SCCACHE_BASEDIRS="${SPUR_ROOT}"
fi

# Use S3 unless explicitly disabled.
enable_spur_s3_cache || true

# ---- per-repo namespace: share the L1 S3 cache with the cloud-build remote --
# The remote builder (scripts/cloud-build/build.sh) writes objects under an S3
# key prefix == the repo namespace (basename of the MAIN repo root, e.g.
# `spur`, `spur-notebook`; see DEFAULT_REMOTE_NAMESPACE there). Mirroring that
# prefix locally makes a repo's local build READ/WRITE the same S3 keys the VM
# produced — genuine local↔remote reuse — while keeping different repos isolated.
#
# sccache bakes SCCACHE_S3_KEY_PREFIX into the server at start-server time, so a
# single shared daemon would apply repo A's prefix to repo B. We therefore give
# each namespace its own server (UDS socket). L0 disk (SCCACHE_DIR) is
# content-addressed and safely shared across namespaces.
if use_spur_s3_sccache && [[ -n "$GIT_ROOT" ]]; then
    NS_MAIN_ROOT=""
    NS_COMMON_DIR=$(git rev-parse --git-common-dir 2>/dev/null || echo "")
    if [[ -n "$NS_COMMON_DIR" ]]; then
        [[ "$NS_COMMON_DIR" != /* ]] && NS_COMMON_DIR="$GIT_ROOT/$NS_COMMON_DIR"
        NS_MAIN_ROOT=$(cd "$NS_COMMON_DIR/.." && pwd -P 2>/dev/null || echo "")
    fi
    [[ -n "$NS_MAIN_ROOT" ]] || NS_MAIN_ROOT="$GIT_ROOT"

    SPUR_NS="${SPUR_SCCACHE_NAMESPACE:-$(basename "$NS_MAIN_ROOT")}"
    export SCCACHE_S3_KEY_PREFIX="${SCCACHE_S3_KEY_PREFIX:-$SPUR_NS}"
    if [[ -z "${SCCACHE_SERVER_UDS:-}" && -z "${SCCACHE_SERVER_PORT:-}" ]]; then
        # macOS homebrew clients ALWAYS connect via TCP 127.0.0.1:4226 and
        # ignore SCCACHE_SERVER_UDS entirely. Exporting the UDS on Darwin only
        # spawns orphan UDS-bound servers that serve no client (observed
        # 2026-09-23: zero-counter orphans while builds churned auto-spawned
        # disk-only TCP servers). Linux (the build VMs) honors the UDS
        # end-to-end, so keep per-namespace sockets there only. Note: with one
        # shared TCP server on a mac, SCCACHE_S3_KEY_PREFIX is baked per server
        # start — the last-started repo's prefix applies until restart.
        if [[ "$(uname -s 2>/dev/null || echo "")" != "Darwin" ]]; then
            SPUR_SRV_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache}"
            mkdir -p "$SPUR_SRV_DIR" 2>/dev/null || true
            export SCCACHE_SERVER_UDS="$SPUR_SRV_DIR/srv-$SPUR_NS.sock"
        fi
    fi
fi

IS_SCCACHE_CONTROL=0
case "${1:-}" in
    --show-stats|--show-adv-stats|--start-server|--stop-server|--zero-stats|\
    --dist-status|--dist-auth|--debug-preprocessor-cache|--package-toolchain)
        IS_SCCACHE_CONTROL=1 ;;
esac

if [[ -n "${CODEX_SANDBOX:-}" && "$IS_SCCACHE_CONTROL" -eq 0 ]]; then
    exec "$@"
fi

if command -v sccache >/dev/null 2>&1; then
    # Cargo has already resolved its artifact paths before invoking rustc, and
    # rustc itself never reads CARGO_TARGET_DIR. But sccache hashes EVERY
    # CARGO_* env var into the Rust cache key (sccache src/compiler/rust.rs,
    # hash step 8), and cloud-build build.sh exports a per-worktree
    # CARGO_TARGET_DIR (/mnt/cargo/targets/<ns>/worktrees/<uuid>) on the VM.
    # Leaving it set makes every compile key diverge per worktree, so the
    # shared L0 disk + L1 S3 cache never hits across .spur/worktrees/* on the
    # spur-builder. Unset it so identical sources share cache entries.
    # (Mirrors spur-notebook scripts/sccache-worktree.sh, commit a77ced09.)
    unset CARGO_TARGET_DIR
    exec sccache "$@"
fi

exec "$@"
