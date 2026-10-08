#!/usr/bin/env bash
# Sync the current worktree to the build VM and run cargo remotely.
#
# Multi-worktree aware: detects whether we're in the main repo or a
# .spur/worktrees/<UUID> worktree, syncs to a matching remote dir, and uses
# per-worktree CARGO_TARGET_DIR + SCCACHE_BASEDIRS so all worktrees share the
# GCS sccache cache through path normalization.
#
# Spot-preemption resilient: the build VM is a spot instance
# (--instance-termination-action=DELETE), so it can vanish mid-build. We detect
# that case after any remote step fails and recover once by re-spinning +
# re-syncing + re-running — see "orchestrate" near the bottom.
#
# Usage:
#   ./build.sh                              # cargo build --release --workspace
#   ./build.sh -- check                     # cargo check
#   ./build.sh --auto-spin -- test ...      # auto-create VM if missing
#   SPUR_BUILD_JOBS=22 ./build.sh -- build  # override default -j 8
#   SPUR_BUILD_MAX_CONCURRENT=3             # local FIFO admission limit
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck disable=SC1091
# shellcheck source=scripts/gcp-build/config.env
source "$SCRIPT_DIR/config.env"

log() { echo "[build] $*" >&2; }

# Exit code reserved for "VM/infrastructure unavailable" — distinct from any
# cargo exit code (cargo uses 1/100/101). spur-cargo uses this to decide
# whether to fall back to local cargo. Anything else is propagated as-is.
INFRA_UNAVAILABLE=200

# Flags may appear in any order before the optional `--` cargo-args separator.
#   --auto-spin       create/start the VM if it is not already RUNNING
#   --frontend-test   disabled after the notebook repo split; run frontend tests
#                     from getspur/spur-notebook.
#   --pnpm            disabled after the notebook repo split; run pnpm from
#                     getspur/spur-notebook.
AUTO_SPIN=0
FRONTEND_TEST=0
PNPM=0
while [[ "${1:-}" == --* ]]; do
    case "$1" in
        --auto-spin)     AUTO_SPIN=1; shift ;;
        --frontend-test) FRONTEND_TEST=1; shift ;;
        --pnpm)          PNPM=1; shift ;;
        --)              shift; break ;;
        *)               break ;;
    esac
done
CARGO_ARGS="${CARGO_ARGS:-${*:-build --release --workspace}}"
# A `run` subcommand executes the built binary ON the VM, so any files it writes
# into the worktree must be synced back to the local tree (the forward sync is
# one-way: local -> VM). Only `run` triggers this; SPUR_RUN_SYNC_BACK=0 opts out.
# Caveat: only paths under the worktree are captured — a binary that writes to an
# out-of-tree path (e.g. /tmp) leaves those outputs on the VM.
RUN_SYNC_BACK=0
if [[ "${CARGO_ARGS%% *}" == "run" && "${SPUR_RUN_SYNC_BACK:-1}" != "0" ]]; then
    RUN_SYNC_BACK=1
fi
# ---- worktree detection ----------------------------------------------------
# Resolve toplevel from the *current* directory so workers invoking us from
# .spur/worktrees/<uuid>/ pick up that worktree's path.
GIT_TOPLEVEL=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
if [[ -z "$GIT_TOPLEVEL" ]]; then
    log "Not inside a git repo. Aborting."
    exit 2
fi
LOCAL_CWD=$(pwd -P)
if [[ "$LOCAL_CWD" == "$GIT_TOPLEVEL" ]]; then
    WORKDIR_REL="."
elif [[ "$LOCAL_CWD" == "$GIT_TOPLEVEL/"* ]]; then
    WORKDIR_REL="${LOCAL_CWD#"$GIT_TOPLEVEL"/}"
else
    log "Current directory is outside git root: $LOCAL_CWD"
    exit 2
fi
WORKDIR_REL_ESCAPED="$(printf '%q' "$WORKDIR_REL")"

if [[ "$GIT_TOPLEVEL" == *"/.spur/worktrees/"* ]]; then
    WORKTREE_UUID=$(basename "$GIT_TOPLEVEL")
    WORKTREE_KEY="worktrees/$WORKTREE_UUID"
else
    WORKTREE_KEY="main"
fi
WORKTREE_FILE_KEY="${WORKTREE_KEY//\//_}"
REMOTE_DIR="spur/$WORKTREE_KEY"                       # e.g. spur/worktrees/UUID
REMOTE_TARGET="/mnt/cargo/targets/$WORKTREE_KEY"
# Run-sync-back scratch paths (VM-side): a marker stamped just before the binary
# runs, and the NUL-delimited list of worktree files newer than it afterwards.
REMOTE_RUN_MARKER="/tmp/spur-run-marker.$WORKTREE_FILE_KEY"
REMOTE_RUN_CHANGED="/tmp/spur-run-changed.$WORKTREE_FILE_KEY"
# Per-invocation PRIVATE run dir (only used for `run` sync-back). A remote `run`
# executes the binary in this private copy of the just-synced worktree instead of
# the shared ~/spur/<key>, so the post-run `find -newer` capture can never pick up
# files a CONCURRENT build wrote into the shared tree. Unique per build.sh process
# ($$ + epoch); the shared compile cache is reused via a target/ symlink.
REMOTE_RUN_DIR="/mnt/cargo/spur-runs/$WORKTREE_FILE_KEY.$$.$(date +%s)"
JOBS="${SPUR_BUILD_JOBS:-8}"
log "Worktree: $WORKTREE_KEY  (local=$GIT_TOPLEVEL)"
log "Remote:   ~/$REMOTE_DIR   target=$REMOTE_TARGET   -j$JOBS"
if [[ "$WORKDIR_REL" != "." ]]; then
    log "Cargo cwd: $WORKDIR_REL"
fi

# ---- local FIFO admission control -----------------------------------------
# The builder VM is shared by every local agent/session. Without admission
# control, many callers can all sync and compile at once, saturating SSH, disk,
# and cargo job slots on the single VM. Keep the gate local: it preserves the
# existing VM-unavailable fallback contract, avoids a long-lived remote lock that
# has to span local rsync calls, and handles the common pressure source.
BUILD_QUEUE_MAX="${SPUR_BUILD_MAX_CONCURRENT:-3}"
BUILD_QUEUE_POLL_SECONDS="${SPUR_BUILD_QUEUE_POLL_SECONDS:-2}"
BUILD_QUEUE_ROOT="${SPUR_BUILD_QUEUE_DIR:-/tmp/spur-gcp-build-queue}"
BUILD_QUEUE_NAME="${GCP_PROJECT}.${GCP_ZONE}.${VM_NAME}"
BUILD_QUEUE_NAME="${BUILD_QUEUE_NAME//[^A-Za-z0-9_.-]/_}"
BUILD_QUEUE_DIR="$BUILD_QUEUE_ROOT/$BUILD_QUEUE_NAME"
BUILD_QUEUE_LOCK_DIR="$BUILD_QUEUE_DIR/.lock"
BUILD_QUEUE_TICKETS_DIR="$BUILD_QUEUE_DIR/tickets"
BUILD_QUEUE_TICKET=""
BUILD_QUEUE_SLOT_ACQUIRED=0
BUILD_QUEUE_LOCK_HELD=0
VM_LIFECYCLE_LOCK_DIR="$BUILD_QUEUE_DIR/vm-lifecycle.lock"
VM_LIFECYCLE_LOCK_HELD=0

queue_lock() {
    mkdir -p "$BUILD_QUEUE_DIR" "$BUILD_QUEUE_TICKETS_DIR"
    while ! mkdir "$BUILD_QUEUE_LOCK_DIR" 2>/dev/null; do
        local owner=""
        if [[ -f "$BUILD_QUEUE_LOCK_DIR/pid" ]]; then
            owner=$(cat "$BUILD_QUEUE_LOCK_DIR/pid" 2>/dev/null || true)
        fi
        if [[ -n "$owner" ]] && ! kill -0 "$owner" 2>/dev/null; then
            rm -rf "$BUILD_QUEUE_LOCK_DIR" 2>/dev/null || true
            continue
        fi
        sleep 0.1
    done
    printf '%s\n' "$$" >"$BUILD_QUEUE_LOCK_DIR/pid"
    BUILD_QUEUE_LOCK_HELD=1
}

queue_unlock() {
    if [[ "$BUILD_QUEUE_LOCK_HELD" -eq 1 ]]; then
        rm -rf "$BUILD_QUEUE_LOCK_DIR" 2>/dev/null || true
        BUILD_QUEUE_LOCK_HELD=0
    fi
}

queue_cleanup_stale_tickets() {
    local ticket pid
    for ticket in "$BUILD_QUEUE_TICKETS_DIR"/*; do
        [[ -f "$ticket" ]] || continue
        pid=$(sed -n '1p' "$ticket" 2>/dev/null || true)
        if [[ -z "$pid" ]] || ! kill -0 "$pid" 2>/dev/null; then
            rm -f "$ticket" 2>/dev/null || true
        fi
    done
}

queue_ticket_rank() {
    local rank=0 ticket name
    while IFS= read -r ticket; do
        [[ -n "$ticket" ]] || continue
        name=$(basename "$ticket")
        rank=$((rank + 1))
        if [[ "$name" == "$BUILD_QUEUE_TICKET" ]]; then
            printf '%s\n' "$rank"
            return 0
        fi
    done < <(
        for ticket in "$BUILD_QUEUE_TICKETS_DIR"/*; do
            [[ -f "$ticket" ]] && printf '%s\n' "$ticket"
        done | sort
    )
    printf '0\n'
}

acquire_build_queue_slot() {
    if ! [[ "$BUILD_QUEUE_MAX" =~ ^[0-9]+$ ]] || [[ "$BUILD_QUEUE_MAX" -eq 0 ]]; then
        log "Remote build queue disabled (SPUR_BUILD_MAX_CONCURRENT=$BUILD_QUEUE_MAX)"
        return 0
    fi

    local counter next rank
    queue_lock
    counter=0
    if [[ -f "$BUILD_QUEUE_DIR/counter" ]]; then
        counter=$(cat "$BUILD_QUEUE_DIR/counter" 2>/dev/null || echo 0)
    fi
    [[ "$counter" =~ ^[0-9]+$ ]] || counter=0
    next=$((counter + 1))
    printf '%s\n' "$next" >"$BUILD_QUEUE_DIR/counter"
    BUILD_QUEUE_TICKET=$(printf '%020d' "$next")
    printf '%s\n%s\n%s\n' "$$" "$(date +%s)" "$WORKTREE_KEY" >"$BUILD_QUEUE_TICKETS_DIR/$BUILD_QUEUE_TICKET"
    queue_unlock

    while :; do
        queue_lock
        queue_cleanup_stale_tickets
        rank=$(queue_ticket_rank)
        if [[ "$rank" -gt 0 && "$rank" -le "$BUILD_QUEUE_MAX" ]]; then
            BUILD_QUEUE_SLOT_ACQUIRED=1
            queue_unlock
            log "Remote build queue: admitted as ticket $next (slot $rank/$BUILD_QUEUE_MAX)."
            return 0
        fi
        queue_unlock
        log "Remote build queue: ticket $next waiting; $BUILD_QUEUE_MAX build(s) already admitted."
        sleep "$BUILD_QUEUE_POLL_SECONDS"
    done
}

release_build_queue_slot() {
    if [[ -n "${BUILD_QUEUE_TICKET:-}" ]]; then
        rm -f "$BUILD_QUEUE_TICKETS_DIR/$BUILD_QUEUE_TICKET" 2>/dev/null || true
        if [[ "$BUILD_QUEUE_SLOT_ACQUIRED" -eq 1 ]]; then
            log "Remote build queue: released ticket $BUILD_QUEUE_TICKET."
        fi
        BUILD_QUEUE_TICKET=""
        BUILD_QUEUE_SLOT_ACQUIRED=0
    fi
}

vm_lifecycle_lock() {
    mkdir -p "$BUILD_QUEUE_DIR"
    while ! mkdir "$VM_LIFECYCLE_LOCK_DIR" 2>/dev/null; do
        local owner=""
        if [[ -f "$VM_LIFECYCLE_LOCK_DIR/pid" ]]; then
            owner=$(cat "$VM_LIFECYCLE_LOCK_DIR/pid" 2>/dev/null || true)
        fi
        if [[ -n "$owner" ]] && ! kill -0 "$owner" 2>/dev/null; then
            rm -rf "$VM_LIFECYCLE_LOCK_DIR" 2>/dev/null || true
            continue
        fi
        log "VM lifecycle: another local build is recovering $VM_NAME; waiting..."
        sleep "$BUILD_QUEUE_POLL_SECONDS"
    done
    printf '%s\n' "$$" >"$VM_LIFECYCLE_LOCK_DIR/pid"
    VM_LIFECYCLE_LOCK_HELD=1
}

vm_lifecycle_unlock() {
    if [[ "$VM_LIFECYCLE_LOCK_HELD" -eq 1 ]]; then
        rm -rf "$VM_LIFECYCLE_LOCK_DIR" 2>/dev/null || true
        VM_LIFECYCLE_LOCK_HELD=0
    fi
}

# ---- VM lifecycle ----------------------------------------------------------
# Distinguish three states:
#   RUNNING                       — proceed
#   TERMINATED / STOPPED / etc.   — `instances start` (cheap, ~30s)
#   MISSING                       — `spin.sh` (creates from scratch)

wait_for_ssh() {
    log "Waiting for SSH on $VM_NAME..."
    for _ in $(seq 1 30); do
        if gcloud compute ssh "$VM_NAME" \
                --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
                --tunnel-through-iap --quiet \
                --command='true' >/dev/null 2>&1; then
            return 0
        fi
        sleep 5
    done
    return 1
}

# Echo the VM's current GCE status, or MISSING if it does not exist / describe
# fails. A deleted spot instance (the preemption outcome with
# --instance-termination-action=DELETE) reports MISSING here.
vm_status() {
    gcloud compute instances describe "$VM_NAME" \
        --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
        --format='value(status)' 2>/dev/null || echo "MISSING"
}

# Bring the VM to RUNNING, honoring AUTO_SPIN. On any unrecoverable condition
# this exits INFRA_UNAVAILABLE so spur-cargo falls back to local cargo. A fresh
# spin.sh recreates an ephemeral Local SSD cache, while GCS sccache is the
# durable compile cache across Spot preemption. The boot disk (and synced source
# tree under ~/spur) is lost and re-synced.
ensure_vm_up() {
    local status
    vm_lifecycle_lock
    status=$(vm_status)
    case "$status" in
        RUNNING)
            ;;
        TERMINATED|STOPPED|STOPPING|SUSPENDED|SUSPENDING)
            if [[ $AUTO_SPIN -eq 1 ]]; then
                log "VM $VM_NAME is $status — starting..."
                if ! gcloud compute instances start "$VM_NAME" \
                    --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
                    --quiet; then
                    status=$(vm_status)
                    if [[ "$status" == "RUNNING" ]]; then
                        log "start failed but VM is now RUNNING; another local build likely recovered it."
                    else
                        log "start failed and VM is $status"
                        exit $INFRA_UNAVAILABLE
                    fi
                fi
                wait_for_ssh || { log "SSH never came up"; exit $INFRA_UNAVAILABLE; }
            else
                log "VM $VM_NAME is $status. Pass --auto-spin to start it."
                exit $INFRA_UNAVAILABLE
            fi
            ;;
        MISSING)
            if [[ $AUTO_SPIN -eq 1 ]]; then
                log "VM $VM_NAME missing — spinning a fresh one..."
                if ! "$SCRIPT_DIR/spin.sh"; then
                    status=$(vm_status)
                    if [[ "$status" == "RUNNING" ]]; then
                        log "spin.sh failed but VM is now RUNNING; another local build likely recovered it."
                    else
                        log "spin.sh failed and VM is $status"
                        exit $INFRA_UNAVAILABLE
                    fi
                fi
                wait_for_ssh || { log "SSH never came up"; exit $INFRA_UNAVAILABLE; }
            else
                log "VM $VM_NAME missing. Pass --auto-spin to create it."
                exit $INFRA_UNAVAILABLE
            fi
            ;;
        *)
            log "VM $VM_NAME in unexpected state: $status"
            exit $INFRA_UNAVAILABLE
            ;;
    esac
    vm_lifecycle_unlock
}

# ---- choose SSH transport: direct (default) -> IAP -> local ----------------
# Direct SSH connects to the VM's external IP and skips the IAP tunnel, which
# roughly doubles upload throughput when the uplink has headroom (measured
# ~1 MB/s over IAP vs ~2.5 MB/s direct from Vietnam->asia-southeast1). It uses
# SPUR_DIRECT_SSH_PORT (default: 22) and falls back to IAP if direct public SSH
# is not reachable. We probe the preferred transport with a cheap `true`
# (bounded by ConnectTimeout so a filtered port fails fast) and fall back in order:
#   direct -> IAP -> exit INFRA_UNAVAILABLE (caller spur-cargo builds locally)
#
# Both modes run through `gcloud compute ssh`, so OS Login, key management, and
# host-key validation are identical — only the transport path differs (omit vs
# pass --tunnel-through-iap). Set SPUR_DIRECT_SSH=0 to force IAP-only.
# Direct probing defaults to a short timeout so filtered public SSH fails fast;
# IAP needs a longer default because tunnel setup can complete TCP connection
# but still need a few more seconds for SSH banner exchange.
DIRECT_SSH_PORT_EFFECTIVE=""
direct_ssh_port() {
    DIRECT_SSH_PORT_EFFECTIVE="${SPUR_DIRECT_SSH_PORT:-22}"
    if [[ ! "$DIRECT_SSH_PORT_EFFECTIVE" =~ ^[0-9]+$ ]]; then
        log "Invalid SPUR_DIRECT_SSH_PORT=$DIRECT_SSH_PORT_EFFECTIVE"
        exit $INFRA_UNAVAILABLE
    fi
}

probe_transport() {
    # $1: transport flag ("" for direct, "--tunnel-through-iap" for IAP).
    local connect_timeout
    if [[ -n "$1" ]]; then
        connect_timeout="${SPUR_IAP_SSH_CONNECT_TIMEOUT:-${SPUR_SSH_CONNECT_TIMEOUT:-10}}"
        gcloud compute ssh "$VM_NAME" \
            --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
            "$1" --quiet --command='true' \
            -- -o ConnectTimeout="$connect_timeout" >/dev/null 2>&1
    else
        connect_timeout="${SPUR_DIRECT_SSH_CONNECT_TIMEOUT:-${SPUR_SSH_CONNECT_TIMEOUT:-3}}"
        direct_ssh_port
        if [[ "$DIRECT_SSH_PORT_EFFECTIVE" != "22" ]]; then
            gcloud compute ssh "$VM_NAME" \
                --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
                --ssh-flag="-p $DIRECT_SSH_PORT_EFFECTIVE" \
                --quiet --command='true' \
                -- -o ConnectTimeout="$connect_timeout" >/dev/null 2>&1
        else
            gcloud compute ssh "$VM_NAME" \
                --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
                --quiet --command='true' \
                -- -o ConnectTimeout="$connect_timeout" >/dev/null 2>&1
        fi
    fi
}

# Pick a working SSH transport and publish it via the IAP_FLAG / TRANSPORT
# globals (re-probed on every dispatch attempt — a fresh post-preemption VM has
# a new external IP). Exits INFRA_UNAVAILABLE when neither transport is
# reachable. Sets the env that the rsync transport (_gcloud-ssh.sh) reads.
choose_transport() {
    local direct="${SPUR_DIRECT_SSH:-1}"
    if [[ "$direct" != "0" ]] && probe_transport ""; then
        IAP_FLAG=""; TRANSPORT_MODE="direct (external IP:${SPUR_DIRECT_SSH_PORT:-22})"
    elif probe_transport "--tunnel-through-iap"; then
        IAP_FLAG="--tunnel-through-iap"; TRANSPORT_MODE="IAP tunnel"
    else
        log "No SSH transport reachable (direct + IAP both failed) — falling back to local build."
        exit $INFRA_UNAVAILABLE
    fi
    log "SSH transport: $TRANSPORT_MODE"

    TRANSPORT="$SCRIPT_DIR/_gcloud-ssh.sh"
    export GCP_PROJECT GCP_ZONE
    # _gcloud-ssh.sh (the rsync transport) reads this to match the chosen mode.
    export SPUR_SSH_IAP_FLAG="$IAP_FLAG" SPUR_DIRECT_SSH_PORT
}

# Every remote command goes through here so it uses the chosen transport.
# $IAP_FLAG is intentionally unquoted: empty -> no arg (direct), else one flag.
remote_ssh() {
    if [[ -z "${IAP_FLAG:-}" ]]; then
        direct_ssh_port
        if [[ "$DIRECT_SSH_PORT_EFFECTIVE" != "22" ]]; then
            gcloud compute ssh "$VM_NAME" \
                --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
                --ssh-flag="-p $DIRECT_SSH_PORT_EFFECTIVE" \
                --quiet "$@"
        else
            gcloud compute ssh "$VM_NAME" \
                --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
                --quiet "$@"
        fi
    else
        gcloud compute ssh "$VM_NAME" \
            --project="$GCP_PROJECT" --zone="$GCP_ZONE" \
            $IAP_FLAG --quiet "$@"
    fi
}

# ---- sync the worktree to the VM -------------------------------------------
# Returns nonzero (without aborting the script) if any sync step fails, so the
# orchestrator can tell a spot preemption mid-sync from a genuine error by
# re-checking VM state. FILE_LIST / XFER_LIST are created once by the caller and
# reused across attempts.
sync_workspace() {
    log "Enumerating git-tracked files..."
    cd "$GIT_TOPLEVEL"
    git ls-files -z --cached --others --exclude-standard >"$FILE_LIST" || return $?
    "$SCRIPT_DIR/_sync-dangling-symlinks.sh" partition \
        "$GIT_TOPLEVEL" \
        "$FILE_LIST" \
        "$RSYNC_FILE_LIST" \
        "$DANGLING_SYMLINK_LIST" || return $?
    local count
    count=$(tr -cd '\0' <"$FILE_LIST" | wc -c | tr -d ' ')
    log "  $count files"

    # Ensure remote parent dir exists (idempotent).
    # Note we symlink the in-source target/ -> $REMOTE_TARGET instead of setting
    # CARGO_TARGET_DIR env — sccache hashes CARGO_TARGET_DIR (it's a CARGO_* var)
    # which would defeat cross-worktree cache sharing.
    #
    # Only (re)create the symlink when it is missing or points elsewhere. An
    # unconditional `rm -rf target && ln -sf` momentarily deletes the link; a
    # concurrent build sharing this worktree key (e.g. another session running
    # cargo against ~/spur/main) then races into the gap and rustc fails with
    # "os error 2" creating a temp dir under target/release/deps. When the link is
    # already correct — the steady state for repeated builds — we touch nothing and
    # the window never opens.
    remote_ssh \
        --command="mkdir -p ~/$REMOTE_DIR $REMOTE_TARGET /mnt/cargo/cargo-home /mnt/cargo/rustup && link=\"\$HOME/$REMOTE_DIR/target\" && if [ \"\$(readlink \"\$link\" 2>/dev/null)\" != \"$REMOTE_TARGET\" ]; then rm -rf \"\$link\"; ln -s \"$REMOTE_TARGET\" \"\$link\"; fi" >/dev/null || return $?

    local remote_xfer_list="/tmp/spur-sync-xfer.$WORKTREE_FILE_KEY"
    SYNC_TRANSFER_COUNT=0
    log "Syncing to $VM_NAME:~/$REMOTE_DIR ..."
    # Capture the exact set of files rsync transfers. --checksum is intentional:
    # the VM-side restamp below makes changed sources newer than any cached cargo
    # artifact, but it also means remote mtimes no longer match local mtimes. If
    # rsync used its default size+mtime quick-check, every warm run would treat
    # restamped files as changed, re-transfer thousands of paths, restamp them
    # again, and force cargo to rebuild workspace crates. Checksum mode makes the
    # delta content-based, and --omit-dir-times keeps directory metadata churn out
    # of $XFER_LIST so only content changes are restamped.
    rsync -azcO --delete -0 --files-from="$RSYNC_FILE_LIST" --out-format='%n' \
        -e "$TRANSPORT" \
        "$GIT_TOPLEVEL/" "$VM_NAME:$REMOTE_DIR/" >"$XFER_LIST" || return $?

    if [[ -s "$DANGLING_SYMLINK_LIST" ]]; then
        local remote_symlink_manifest="/tmp/spur-sync-symlinks.$WORKTREE_FILE_KEY"
        local remote_symlink_helper="/tmp/spur-sync-symlinks.$WORKTREE_FILE_KEY.sh"
        local symlink_count
        symlink_count=$(($(tr -cd '\0' <"$DANGLING_SYMLINK_LIST" | wc -c | tr -d ' ') / 2))
        log "Reconstructing $symlink_count tracked dangling symlink(s)..."
        rsync -az -e "$TRANSPORT" \
            "$DANGLING_SYMLINK_LIST" "$VM_NAME:$remote_symlink_manifest" || return $?
        rsync -az -e "$TRANSPORT" \
            "$SCRIPT_DIR/_sync-dangling-symlinks.sh" "$VM_NAME:$remote_symlink_helper" || return $?
        remote_ssh \
            --command="bash \"$remote_symlink_helper\" restore \"$REMOTE_DIR\" \"$remote_symlink_manifest\"" \
            >>"$XFER_LIST" || return $?
    fi

    # ---- normalize mtimes of just-synced files to the VM clock -------------
    # Cargo decides whether to rebuild a path dependency (e.g. jute) by comparing
    # source-file mtimes against the cached artifact's mtime — not by content hash.
    # rsync -a (-> -t) preserves the dev machine's mtimes on the VM copy. When the
    # dev clock lags the VM clock even slightly, a freshly edited file lands with an
    # mtime OLDER than an artifact the VM built moments earlier, so cargo declares
    # the crate fresh and silently skips the edit (observed: a newly added method
    # never compiled in despite a clean source sync). Touching exactly the
    # transferred files to the VM's "now" guarantees changed sources are newer than
    # any prior artifact (correct rebuild) while leaving unchanged files untouched
    # (incremental cache intact). touch -c skips anything rsync reported but that no
    # longer exists (e.g. directory entries that were pruned).
    # See: docs/rca/2026-05-31-remote-cargo-stale-fingerprint.md
    if [[ -s "$XFER_LIST" ]]; then
        SYNC_TRANSFER_COUNT=$(wc -l <"$XFER_LIST" | tr -d ' ')
        log "Re-stamping $SYNC_TRANSFER_COUNT synced path(s) to VM clock..."
        rsync -az -e "$TRANSPORT" "$XFER_LIST" "$VM_NAME:$remote_xfer_list" || return $?
        remote_ssh \
            --command="cd \"\$HOME/$REMOTE_DIR\" && while IFS= read -r f; do [ -n \"\$f\" ] && touch -c -- \"\$f\"; done < \"$remote_xfer_list\"" || return $?
    fi

    # ---- reconcile: prune files deleted locally since the last sync --------
    # rsync's --delete is INERT when combined with --files-from (it transfers only
    # the listed files and never recurses the destination, so it cannot prune
    # extraneous files). A file removed/renamed locally therefore lingers on the VM
    # and silently desyncs the remote build from local (stale modules, removed
    # tests). We ship the current manifest and let a VM-side helper delete exactly
    # (previous manifest - current manifest): only ever rsync-managed source that is
    # now gone, never VM-generated artifacts (node_modules/, dist/, target/) which
    # were never in any manifest. The baseline manifest persists under /mnt/cargo
    # keyed by worktree for the lifetime of the current Local SSD VM.
    local remote_manifest_cur="/tmp/spur-sync-manifest.$WORKTREE_FILE_KEY"
    local stored_manifest="/mnt/cargo/sync-manifests/$WORKTREE_KEY.manifest"
    log "Reconciling remote workspace (pruning locally-deleted files)..."
    rsync -az -e "$TRANSPORT" "$FILE_LIST" "$VM_NAME:$remote_manifest_cur" || return $?
    remote_ssh \
        --command="bash \"\$HOME/$REMOTE_DIR/scripts/gcp-build/_prune-remote.sh\" \"$REMOTE_DIR\" \"$remote_manifest_cur\" \"$stored_manifest\"" || return $?
}

# ---- run the requested payload on the VM -----------------------------------
# Returns the remote command's exit code (without aborting the script) so the
# orchestrator can distinguish a genuine build/test failure from a preemption.
run_payload() {
    if [[ $PNPM -eq 1 ]]; then
        log "--pnpm is disabled in getspur/SpurCode after the notebook repo split."
        log "Run pnpm from a getspur/spur-notebook checkout instead."
        return 2
    fi
    if [[ $FRONTEND_TEST -eq 1 ]]; then
        log "--frontend-test is disabled in getspur/SpurCode after the notebook repo split."
        log "Run frontend tests from a getspur/spur-notebook checkout instead."
        return 2
    fi

    # ---- run cargo on the VM ----------------------------------------------
    # Forward caller's $RUSTFLAGS so cfg/lint flags survive the SSH hop. The
    # remote bash re-parses `cargo $CARGO_ARGS` and would strip quotes from a
    # `--config build.rustflags=[...]` arg, so we propagate via env instead and
    # escape with %q to survive both the local→gcloud quoting and the remote
    # bash -lc re-parse.
    #
    # Only emit the export when the caller actually set RUSTFLAGS. A set-but-empty
    # RUSTFLAGS is NOT inert: cargo treats it as "no flags" and IGNORES the synced
    # .cargo/config.toml `build.rustflags`, silently dropping
    # `-C force-frame-pointers=yes`. That both diverges from local builds (where
    # config applies) and changes rustc args, fragmenting the shared sccache cache.
    # Verified: `RUSTFLAGS='' cargo build -v` omits force-frame-pointers; unset
    # preserves it. So when unset we emit no export at all and let config win.
    local remote_rustflags_export=""
    if [[ -n "${RUSTFLAGS:-}" ]]; then
        remote_rustflags_export="export RUSTFLAGS=$(printf '%q' "$RUSTFLAGS")"
    fi
    log "Running: cargo $CARGO_ARGS  -j$JOBS${RUSTFLAGS:+  RUSTFLAGS=$RUSTFLAGS}"
    local capture_cargo_output=0
    if [[ "${SPUR_CAPTURE_FRESH_CARGO_OUTPUT:-1}" != "0" && "${SYNC_TRANSFER_COUNT:-0}" == "0" ]]; then
        capture_cargo_output=1
        log "No source content delta; capturing cargo output on VM (set SPUR_CAPTURE_FRESH_CARGO_OUTPUT=0 to stream)."
    fi
    # For a remote `run`, stamp a marker immediately before the binary executes,
    # then record every worktree file newer than it (excluding build artifacts).
    # %P prints worktree-relative paths so the list feeds rsync --files-from on
    # the way back. Empty strings when sync-back is off (any non-run subcommand).
    local run_setup_cmd="" run_marker_cmd="" run_capture_cmd=""
    if [[ $RUN_SYNC_BACK -eq 1 ]]; then
        # Isolation: copy the just-synced shared worktree into a PRIVATE per-run
        # dir and cd into it, so the binary executes where no concurrent build can
        # write — the post-run `find` capture then sees only THIS run's outputs.
        # The compile cache is reused via a target/ symlink to the shared
        # /mnt/cargo/targets/<key> (target/ is excluded from the copy and capture).
        # Old run dirs (>2h) are swept opportunistically so they don't accumulate.
        run_setup_cmd="mkdir -p /mnt/cargo/spur-runs; find /mnt/cargo/spur-runs -maxdepth 1 -type d -mmin +120 -exec rm -rf {} + 2>/dev/null || true; rsync -a --delete --exclude /target --exclude /.git \"\$HOME/$REMOTE_DIR/\" \"$REMOTE_RUN_DIR/\" && ln -sfn \"$REMOTE_TARGET\" \"$REMOTE_RUN_DIR/target\" && cd \"$REMOTE_RUN_DIR\""
        run_marker_cmd="touch \"$REMOTE_RUN_MARKER\""
        # Capture worktree files the binary wrote. Exclude build artifacts
        # (target/), git internals (.git/), and nested worktrees
        # (.spur/worktrees/ — each has its own target/ and must never sync), but
        # KEEP other .spur outputs (e.g. .spur/analyst.duckdb from `analyst
        # build`) so graph-producing runs land their DB locally.
        # NOTE: this string is embedded inside a remote `bash -lc '...'` (single
        # quotes). Path patterns MUST use escaped double quotes, not single
        # quotes — a literal ' would close the bash -lc wrapper early and the
        # pattern would glob-expand on the VM (CWD has target/), breaking find.
        run_capture_cmd="(cd \"$REMOTE_RUN_DIR\" && find . -type f -newer \"$REMOTE_RUN_MARKER\" -not -path \"./target/*\" -not -path \"./.git/*\" -not -path \"./.spur/worktrees/*\" -printf \"%P\\0\" > \"$REMOTE_RUN_CHANGED\") || true"
    fi
    remote_ssh \
        --command="bash -lc '
            set -e
            cd ~/$REMOTE_DIR
            source /etc/profile.d/spur-build.sh 2>/dev/null || true
            # Big-disk TMPDIR. Nested builds that use std::env::temp_dir() — e.g. the
            # rest-table-gateway-ext load/action harnesses, which point
            # CARGO_TARGET_DIR at a ~6G DuckDB build under a temp dir — would
            # otherwise land on the small ~49G root disk that holds /tmp and exhaust
            # it. /mnt/cargo is the ~295G build disk. TMPDIR is not a CARGO_* var, so
            # this does NOT affect sccache cross-worktree hashing.
            mkdir -p /mnt/cargo/tmp
            export TMPDIR=/mnt/cargo/tmp
            # Do NOT set CARGO_TARGET_DIR — sccache hashes CARGO_* env vars, which
            # would defeat cross-worktree sharing. We symlinked target/ to
            # /mnt/cargo/targets/\$WORKTREE_KEY at sync time instead. Likewise
            # SCCACHE_BASEDIRS is set per rustc invocation by the
            # sccache-worktree wrapper (RUSTC_WRAPPER in profile.d).
            export CARGO_BUILD_JOBS=$JOBS
            $remote_rustflags_export
            sccache --start-server >/dev/null 2>&1 || true
            cargo_log=/tmp/spur-cargo-output.$WORKTREE_FILE_KEY.log
            $run_setup_cmd
            remote_workdir_rel=$WORKDIR_REL_ESCAPED
            if [[ \"\$remote_workdir_rel\" != \".\" ]]; then
                cd \"\$remote_workdir_rel\"
            fi
            $run_marker_cmd
            if [[ $capture_cargo_output -eq 1 ]]; then
                set +e
                cargo $CARGO_ARGS >\"\$cargo_log\" 2>&1
                cargo_rc=\$?
                set -e
                if [[ \$cargo_rc -ne 0 ]]; then
                    cat \"\$cargo_log\"
                    exit \$cargo_rc
                fi
                tail -30 \"\$cargo_log\"
            else
                cargo $CARGO_ARGS
            fi
            $run_capture_cmd
            echo
            echo \"--- sccache stats ($WORKTREE_KEY) ---\"
            sccache --show-stats > /tmp/spur-sccache-stats.$WORKTREE_FILE_KEY
            sed -n '1,20p' /tmp/spur-sccache-stats.$WORKTREE_FILE_KEY
        '" || return $?

    log "Done. target lives at $VM_NAME:$REMOTE_TARGET — use scripts/gcp-build/fetch.sh to pull artifacts."
    return 0
}

# ---- sync run outputs back from the VM -------------------------------------
# A remote `run` executes the binary on the VM, so worktree files it created or
# modified live only there. run_payload recorded them in $REMOTE_RUN_CHANGED
# (NUL-delimited, worktree-relative); pull exactly those paths into the local
# worktree. No --delete: we only add/update, never remove local files. Returns
# nonzero on transfer failure so the orchestrator can tell a preemption (VM gone)
# from a genuine error (VM still RUNNING).
sync_back_workspace() {
    [[ $RUN_SYNC_BACK -eq 1 ]] || return 0
    local rc=0 list
    list=$(mktemp)
    if rsync -az -e "$TRANSPORT" "$VM_NAME:$REMOTE_RUN_CHANGED" "$list" 2>/dev/null && [[ -s "$list" ]]; then
        local count
        count=$(tr -cd '\0' <"$list" | wc -c | tr -d ' ')
        log "Run sync-back: pulling $count worktree path(s) from $VM_NAME -> $GIT_TOPLEVEL ..."
        # Pull from the PRIVATE run dir (isolated copy), not the shared tree, so we
        # only ever land this run's own outputs. No --delete: add/update only.
        rsync -azc --from0 --files-from="$list" -e "$TRANSPORT" \
            "$VM_NAME:$REMOTE_RUN_DIR/" "$GIT_TOPLEVEL/" || rc=$?
    else
        log "Run sync-back: binary wrote no worktree files."
    fi
    rm -f "$list"
    # Remove the private run dir (best-effort; rm drops the target/ symlink, never
    # the shared cache it points at). The 2h sweep in run_setup_cmd catches any
    # stragglers left by a crash/preemption.
    remote_ssh --command="rm -rf -- \"$REMOTE_RUN_DIR\"" >/dev/null 2>&1 || true
    return $rc
}

# ---- orchestrate: ensure VM, sync, run — with one preemption retry ----------
# The build VM is a spot instance created with
# --instance-termination-action=DELETE, so a preemption mid-build deletes the VM
# and drops our SSH session. The failing remote step then exits nonzero
# (typically 255 for a dropped connection) — a code that is neither 0 nor
# INFRA_UNAVAILABLE, so without this handling spur-cargo would misread it as a
# genuine build/test failure and propagate it (and the CLAUDE.md contract tells
# agents NOT to re-run a "real" red remotely).
#
# So after any nonzero step we re-check the VM's state:
#   * vanished (not RUNNING) -> spot preemption. GCS sccache survived, so we
#     re-spin (fresh boot disk and Local SSD -> source tree/cache gone), re-sync,
#     and run once more. The retry is warm at the compile-object layer because
#     sccache serves objects from GCS. A second vanish,
#     or a VM we can't bring up, yields INFRA_UNAVAILABLE so spur-cargo falls
#     back to local cargo.
#   * still RUNNING -> a genuine build/test failure; propagate the exit code
#     unchanged ("a red remote test is a real failure").
FILE_LIST=""
XFER_LIST=""
RSYNC_FILE_LIST=""
DANGLING_SYMLINK_LIST=""
cleanup() {
    release_build_queue_slot
    vm_lifecycle_unlock
    queue_unlock
    [[ -n "${FILE_LIST:-}" ]] && rm -f "$FILE_LIST"
    [[ -n "${XFER_LIST:-}" ]] && rm -f "$XFER_LIST"
    [[ -n "${RSYNC_FILE_LIST:-}" ]] && rm -f "$RSYNC_FILE_LIST"
    [[ -n "${DANGLING_SYMLINK_LIST:-}" ]] && rm -f "$DANGLING_SYMLINK_LIST"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT TERM

acquire_build_queue_slot

FILE_LIST=$(mktemp)
XFER_LIST=$(mktemp)
RSYNC_FILE_LIST=$(mktemp)
DANGLING_SYMLINK_LIST=$(mktemp)

ensure_vm_up
choose_transport

attempt=1
MAX_ATTEMPTS=2
while : ; do
    rc=0
    sync_workspace || rc=$?
    if [[ $rc -eq 0 ]]; then
        run_payload || rc=$?
    fi
    if [[ $rc -eq 0 ]]; then
        sync_back_workspace || rc=$?
    fi
    if [[ $rc -eq 0 ]]; then
        exit 0
    fi

    # Nonzero. Disambiguate spot preemption from a genuine failure by VM state.
    status_after=$(vm_status)
    if [[ "$status_after" == "RUNNING" ]]; then
        # VM is alive — this is a real build/test failure. Propagate as-is.
        exit "$rc"
    fi

    if [[ $attempt -lt $MAX_ATTEMPTS ]]; then
        log "Remote step exited $rc and VM $VM_NAME is now $status_after — likely spot preemption mid-build. Re-spinning + re-syncing and retrying (attempt $((attempt + 1))/$MAX_ATTEMPTS)..."
        attempt=$((attempt + 1))
        AUTO_SPIN=1                 # a recovery spin is always warranted
        ensure_vm_up                # exits INFRA_UNAVAILABLE if it can't spin
        choose_transport            # re-probe — fresh VM has a new external IP
        continue
    fi

    log "VM $VM_NAME unavailable ($status_after) after $MAX_ATTEMPTS attempt(s) — signaling INFRA_UNAVAILABLE ($INFRA_UNAVAILABLE) for local fallback."
    exit $INFRA_UNAVAILABLE
done
