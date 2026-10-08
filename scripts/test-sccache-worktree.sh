#!/usr/bin/env bash
# test-sccache-worktree.sh — behavior test for scripts/sccache-worktree.sh
#
# Runs the REAL wrapper against a stub `sccache` binary and asserts:
#   1. argv passthrough to sccache
#   2. SCCACHE_BASEDIRS set to <git-worktree-root>:<spur-repo-root> (longest-prefix
#      stripping contract) when run inside a git worktree
#   3. CARGO_TARGET_DIR is NOT visible to sccache — sccache hashes every
#      CARGO_* env var into the Rust cache key (sccache src/compiler/rust.rs,
#      hash step 8), and cloud-build build.sh sets a per-worktree
#      CARGO_TARGET_DIR on the VM. Without unsetting it, every compile key
#      diverges per worktree and the shared L0/L1 cache never hits across
#      .spur/worktrees/* on the spur-builder.
#   4. default backend (SPUR_SCCACHE_S3 unset) stays aligned with the aws-my
#      primary builder: two-level disk,s3 against the Malaysia bucket.
#
# Usage: scripts/test-sccache-worktree.sh   (exit 0 = pass, 1 = fail)
set -uo pipefail

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
[[ "$SCRIPT_DIR" == "$SCRIPT_PATH" ]] && SCRIPT_DIR="."
SCRIPT_DIR=$(cd "$SCRIPT_DIR" && pwd -P)
WRAPPER="$SCRIPT_DIR/sccache-worktree.sh"
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd -P)

FAILURES=0
pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1" >&2; FAILURES=$((FAILURES + 1)); }

TMP=$(mktemp -d /tmp/spur-sccache-wrapper-test.XXXXXX)
trap 'rm -rf "$TMP"' EXIT

# ---- fixture: a git repo with one linked worktree --------------------------
MAIN="$TMP/mainrepo"
TMP=$(cd "$TMP" && pwd -P)   # canonical: wrapper compares git worktree paths (pwd -P)
mkdir -p "$MAIN"
git -C "$MAIN" init -q
git -C "$MAIN" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
git -C "$MAIN" worktree add -q "$TMP/wt-abc" -b probe-branch 2>/dev/null
WT="$TMP/wt-abc"

# ---- stub sccache: dump env + argv, always succeed --------------------------
STUB="$TMP/sccache"
cat > "$STUB" <<'EOF'
#!/usr/bin/env bash
{ printf 'argv=%s\n' "$*"
  printf 'basedirs=%s\n' "${SCCACHE_BASEDIRS:-}"
  printf 'target_dir=%s\n' "${CARGO_TARGET_DIR:-}"
  printf 'bucket=%s\n' "${SCCACHE_BUCKET:-}"
  printf 'chain=%s\n' "${SCCACHE_MULTILEVEL_CHAIN:-}"
  printf 'srv_uds=%s\n' "${SCCACHE_SERVER_UDS:-}"
  printf 'idle=%s\n' "${SCCACHE_IDLE_TIMEOUT:-}"
} >> "$SCCACHE_CAPTURE"
exit 0
EOF
chmod 0755 "$STUB"

CAPTURE="$TMP/capture.txt"
export SCCACHE_CAPTURE="$CAPTURE"
export SCCACHE_DIR="$TMP/sccache-dir"          # hermetic L0 (no ~/Library writes)
export PATH="$TMP:$PATH"
unset SPUR_SCCACHE_S3 SPUR_SCCACHE_GCS SPUR_SCCACHE_NAMESPACE SPUR_ROOT || true

run_wrapper() { # $1 = cwd, rest = argv
    local cwd="$1"; shift
    ( cd "$cwd" && "$WRAPPER" "$@" ) >/dev/null 2>&1
}

# ---- case 1..3: worktree invocation shape -----------------------------------
: > "$CAPTURE"
run_wrapper "$WT" rustc --crate-name probe --crate-type lib src/lib.rs

if [[ -s "$CAPTURE" ]]; then
    argv=$(sed -n 's/^argv=//p' "$CAPTURE")
    basedirs=$(sed -n 's/^basedirs=//p' "$CAPTURE")
    target_dir=$(sed -n 's/^target_dir=//p' "$CAPTURE")
    bucket=$(sed -n 's/^bucket=//p' "$CAPTURE")
    chain=$(sed -n 's/^chain=//p' "$CAPTURE")

    [[ "$argv" == "rustc --crate-name probe --crate-type lib src/lib.rs" ]] \
        && pass "argv passed through to sccache" \
        || fail "argv passthrough: got '$argv'"

    # git rev-parse spells the worktree root logically; canonicalize before compare
    basedirs_wt=$(cd "${basedirs%%:*}" 2>/dev/null && pwd -P)
    [[ "$basedirs_wt:${basedirs#*:}" == "$WT:$REPO_ROOT" ]] \
        && pass "SCCACHE_BASEDIRS=<worktree>:<repo-root>" \
        || fail "SCCACHE_BASEDIRS: got '$basedirs', want '$WT:$REPO_ROOT'"

    if [[ -z "$target_dir" ]]; then
        pass "CARGO_TARGET_DIR unset in sccache env (shared keys across worktrees)"
    else
        fail "CARGO_TARGET_DIR leaked to sccache: '$target_dir' — every VM compile key diverges per worktree (sccache hashes all CARGO_* vars)"
    fi

    [[ "$bucket" == "spurlab-591950085580-spur-sccache-apse5" ]] \
        && pass "default L1 bucket = aws-my Malaysia (spurlab-591950085580-spur-sccache-apse5)" \
        || fail "default bucket: got '$bucket', want spurlab-591950085580-spur-sccache-apse5 (aws-my)"

    [[ "$chain" == "disk,s3" ]] \
        && pass "default SCCACHE_MULTILEVEL_CHAIN=disk,s3" \
        || fail "default chain: got '$chain', want disk,s3"
else
    fail "wrapper did not exec the stub sccache (no capture)"
fi

# ---- case 4: CARGO_TARGET_DIR set in caller env (VM shape: build.sh export) --
: > "$CAPTURE"
( cd "$WT" && CARGO_TARGET_DIR=/mnt/cargo/targets/spur/worktrees/uuid1 "$WRAPPER" rustc - --crate-name x ) \
    >/dev/null 2>&1
target_dir=$(sed -n 's/^target_dir=//p' "$CAPTURE")
if [[ -z "$target_dir" ]]; then
    pass "per-worktree CARGO_TARGET_DIR (VM shape) stays out of the sccache env"
else
    fail "VM-shaped CARGO_TARGET_DIR leaked: '$target_dir'"
fi

# ---- case 5: S3 opt-out keeps env clean of S3 vars ---------------------------
: > "$CAPTURE"
( cd "$WT" && SPUR_SCCACHE_S3=0 "$WRAPPER" rustc - --crate-name x ) >/dev/null 2>&1
bucket=$(sed -n 's/^bucket=//p' "$CAPTURE")
chain=$(sed -n 's/^chain=//p' "$CAPTURE")
[[ -z "$bucket" && -z "$chain" ]] \
    && pass "SPUR_SCCACHE_S3=0 disables S3 backend exports" \
    || fail "SPUR_SCCACHE_S3=0 still exported bucket='$bucket' chain='$chain'"

# ---- case 6: stale Google environment must not override AWS ----------------
: > "$CAPTURE"
( cd "$WT" && SCCACHE_GCS_BUCKET=retired-cache SCCACHE_MULTILEVEL_CHAIN=disk,gcs "$WRAPPER" rustc - --crate-name x ) \
    >/dev/null 2>&1
bucket=$(sed -n 's/^bucket=//p' "$CAPTURE")
chain=$(sed -n 's/^chain=//p' "$CAPTURE")
[[ "$bucket" == "spurlab-591950085580-spur-sccache-apse5" && "$chain" == "disk,s3" ]] \
    && pass "stale Google cache settings leave AWS selected" \
    || fail "AWS cache selection failed bucket='$bucket' chain='$chain'"

# ---- case 7: explicit SPUR_SCCACHE_S3=1 outranks ambient GCS config ----------
: > "$CAPTURE"
( cd "$WT" && SCCACHE_GCS_BUCKET=wiilearn-spur-sccache-asia SPUR_SCCACHE_S3=1 "$WRAPPER" rustc - --crate-name x ) \
    >/dev/null 2>&1
bucket=$(sed -n 's/^bucket=//p' "$CAPTURE")
[[ "$bucket" == "spurlab-591950085580-spur-sccache-apse5" ]] \
    && pass "explicit SPUR_SCCACHE_S3=1 wins over ambient GCS config" \
    || fail "explicit S3 override broken under ambient GCS: bucket='$bucket'"

# ---- case 8: macOS server-binding hygiene ------------------------------------
# macOS (homebrew) sccache clients IGNORE SCCACHE_SERVER_UDS and always connect
# via TCP 4226. Exporting the UDS there only spawns orphan UDS-bound servers that
# serve nobody while disk-only TCP servers churn via auto-spawn. On Darwin the
# wrapper must NOT export the UDS, and must pin the idle timeout so a
# multilevel-configured server stays resident between builds.
: > "$CAPTURE"
( cd "$WT" && "$WRAPPER" rustc - --crate-name x ) >/dev/null 2>&1
srv_uds=$(sed -n 's/^srv_uds=//p' "$CAPTURE")
idle=$(sed -n 's/^idle=//p' "$CAPTURE")
if [[ "$(uname)" == "Darwin" ]]; then
    if [[ -z "$srv_uds" ]]; then
        pass "Darwin: no SCCACHE_SERVER_UDS export (clients use TCP 4226)"
    else
        fail "Darwin: SCCACHE_SERVER_UDS exported ('$srv_uds') — orphan-UDS server bug"
    fi
else
    [[ -n "$srv_uds" ]] && pass "non-Darwin: UDS export preserved ($srv_uds)" \
        || fail "non-Darwin: expected UDS export, got none"
fi
[[ "$idle" == "0" ]] \
    && pass "SCCACHE_IDLE_TIMEOUT=0 (multilevel server stays resident)" \
    || fail "idle timeout not pinned: got '${idle:-unset}'"

echo
if [[ $FAILURES -eq 0 ]]; then
    echo "OK — all sccache-worktree wrapper assertions passed"
    exit 0
fi
echo "$FAILURES assertion(s) failed" >&2
exit 1
