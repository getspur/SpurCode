#!/usr/bin/env bash
# Publish the cloud-build scripts as an S3 bundle for CI.
#
# scripts/cloud-build is a git-tracked SYMLINK into the sibling spur-notebook
# checkout (../../spur-notebook/scripts/cloud-build) — on a GitHub runner
# there is nothing behind it. release-dist.yml restores this bundle to the
# sibling path on the runner so the symlink resolves there exactly like it
# does on a workstation. Same pattern as the zigbuild macOS SDK bundle and
# the e2e VM toolchain provisioner, which also live in the sccache bucket.
#
# Re-run this whenever the cloud-build scripts change (it snapshots the
# WORKING TREE of the sibling checkout — the same bytes local dispatches
# use — not a git ref).
#
# *.local.env files are excluded on purpose: CI must take its golden AMI
# from the SPUR_BUILDER_AMI_ID repo variable, not from a workstation-local
# override (config-aws-my.local.env would silently overwrite the workflow's
# AWS_AMI_ID env because the local file is sourced with a plain export).
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
log() { echo "[publish-bundle] $*" >&2; }

SRC="$(cd -- "$SCRIPT_DIR/cloud-build" && pwd -P)" # resolves the symlink
[[ -x "$SRC/build.sh" ]] || { log "no build.sh under $SRC — is the spur-notebook sibling checkout present?"; exit 1; }

# shellcheck disable=SC1091
source "$SCRIPT_DIR/cloud-build/config.env" # SCCACHE_BUCKET / regions

S3_URI="s3://${SCCACHE_BUCKET}/ci/cloud-build/bundle.tar.gz"
S3_REGION="${SCCACHE_S3_REGION:-$AWS_REGION}"

tmp="$(mktemp /tmp/spur-cloud-build-bundle.XXXXXX.tar.gz)"
trap 'rm -f "$tmp"' EXIT
tar -czf "$tmp" -C "$(dirname "$SRC")" --exclude='*.local.env' --exclude='__pycache__' --exclude='.pytest_cache' cloud-build

if tar -tzf "$tmp" | grep -q 'local\.env'; then
    log "bundle unexpectedly contains a *.local.env file — aborting"
    exit 1
fi

log "uploading $(du -h "$tmp" | cut -f1 | tr -d ' ') to $S3_URI ($S3_REGION)"
aws s3 cp "$tmp" "$S3_URI" --region "$S3_REGION" >/dev/null
log "published $S3_URI"
