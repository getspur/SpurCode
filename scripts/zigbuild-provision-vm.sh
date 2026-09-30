#!/usr/bin/env bash
# Provision the cloud-build VM for macOS (aarch64-apple-darwin) cross-compiles
# driven by `scripts/spur-cargo zigbuild`.
#
# What it does:
#   1. Copies a minimal macOS "linker SDK" from THIS Mac to the VM: every
#      .tbd link stub (Frameworks + PrivateFrameworks + usr/lib, symlinks
#      materialized with -L so nothing dangles), usr/include headers, and
#      libclang_rt.osx.a (Apple compiler-rt; pyke's prebuilt onnxruntime
#      links it explicitly). Total ~70 MB.
#   2. On the VM: adds the aarch64-apple-darwin rust-std to EVERY installed
#      toolchain (the repo pins 1.94.1 via rust-toolchain.toml — adding only
#      to `stable` leaves the pinned toolchain without a darwin std and the
#      build dies with E0463), installs zig (latest stable < 0.16) and
#      cargo-zigbuild + bindgen-cli, and appends SDKROOT / darwin-scoped
#      CFLAGS to /etc/profile.d/spur-build.sh.
#   3. Pre-generates libproc's darwin bindings against the synced SDK and
#      installs a VM-side helper to plant them into libproc's OUT_DIR —
#      libproc 0.14's build.rs is host-cfg-gated, so cross builds never
#      generate them (see the POC doc, "libproc" section).
#
# Run from a Mac with the Xcode Command Line Tools installed. Idempotent —
# safe to re-run. AWS keeps SDK/Zig/bindings on EBS for stop/resume; publish
# the bundle so replacement VMs can restore them from S3.
#
# See: docs/superpowers/specs/2026-07-07-zigbuild-macos-cross-poc.md
set -euo pipefail

SCRIPT_PATH="${BASH_SOURCE[0]}"
SCRIPT_DIR="${SCRIPT_PATH%/*}"
[[ "$SCRIPT_DIR" == "$SCRIPT_PATH" ]] && SCRIPT_DIR="."
SCRIPT_DIR=$(cd "$SCRIPT_DIR" && pwd -P)
# Captured before SCRIPT_DIR is repointed at the cloud-build dir below.
SPUR_REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd -P)

log() { echo "[zigbuild-provision] $*" >&2; }

# --publish-bundle: after provisioning, pack /mnt/cargo/macsdk + zig +
# cargo-zigbuild/bindgen binaries into a tarball and upload it to the sccache
# S3 bucket. cloud-build's startup-aws.sh restores that bundle on every fresh
# spot VM at boot, so macOS cross works with no Mac in the loop.
PUBLISH_BUNDLE=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --publish-bundle) PUBLISH_BUNDLE=1; shift ;;
        *) log "unknown option: $1"; exit 2 ;;
    esac
done

# ---- resolve the cloud-build pipeline (same search order as spur-cargo) ----
resolve_cloud_build_dir() {
    local candidate git_toplevel repo_root notebook_repo
    if [[ -n "${SPUR_CLOUD_BUILD_SH:-}" ]]; then
        candidate="$(dirname "$SPUR_CLOUD_BUILD_SH")"
        [[ -d "$candidate" ]] && { printf '%s\n' "$candidate"; return 0; }
        return 1
    fi
    candidate="$SCRIPT_DIR/cloud-build"
    [[ -e "$candidate/build.sh" ]] && { printf '%s\n' "$candidate"; return 0; }
    if [[ -n "${SPUR_NOTEBOOK_REPO:-}" ]]; then
        candidate="$SPUR_NOTEBOOK_REPO/scripts/cloud-build"
        [[ -e "$candidate/build.sh" ]] && { printf '%s\n' "$candidate"; return 0; }
    fi
    git_toplevel=$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel 2>/dev/null || true)
    [[ -n "$git_toplevel" ]] || return 1
    if [[ "$git_toplevel" == *"/.spur/worktrees/"* ]]; then
        repo_root="$(dirname "$(dirname "$(dirname "$git_toplevel")")")"
    else
        repo_root="$git_toplevel"
    fi
    notebook_repo="$(dirname "$repo_root")/spur-notebook"
    candidate="$notebook_repo/scripts/cloud-build"
    [[ -e "$candidate/build.sh" ]] && { printf '%s\n' "$candidate"; return 0; }
    return 1
}

CB="$(resolve_cloud_build_dir)" || {
    log "cannot find scripts/cloud-build (checked SPUR_CLOUD_BUILD_SH, sibling spur-notebook checkout)"
    exit 2
}
# provider_choose_transport expects SCRIPT_DIR to be the cloud-build dir.
SCRIPT_DIR="$CB"
# shellcheck disable=SC1091
source "$CB/config.env"
# shellcheck disable=SC1090
source "$CB/provider-${SPUR_CLOUD}.sh"
provider_choose_transport
log "VM: $REMOTE_HOST via $TRANSPORT_MODE"

remote_ssh() { provider_remote_ssh "$@"; }

# ---- local SDK bits ---------------------------------------------------------
SDK="$(xcrun --show-sdk-path)"
[[ -d "$SDK" ]] || { log "no macOS SDK (xcrun --show-sdk-path failed)"; exit 2; }
log "Local SDK: $SDK ($(xcrun --show-sdk-version))"

clang_rt_candidates=(/Library/Developer/CommandLineTools/usr/lib/clang/*/lib/darwin/libclang_rt.osx.a)
CLANG_RT=""
for c in "${clang_rt_candidates[@]}"; do
    [[ -f "$c" ]] && { CLANG_RT="$c"; break; }
done
[[ -n "$CLANG_RT" ]] || { log "libclang_rt.osx.a not found under CommandLineTools"; exit 2; }

REMOTE_SDK=/mnt/cargo/macsdk/MacOSX.sdk

log "Enumerating SDK .tbd link stubs..."
TBD_LIST=$(mktemp)
trap 'rm -f "$TBD_LIST"' EXIT
(
    cd "$SDK"
    # New SDKs relocate frameworks (including WebKit) into Cryptexes and leave
    # directory symlinks here. Enumerate through those links before rsync -L.
    find -L System/Library/Frameworks -name '*.tbd'
    find -L System/Library/PrivateFrameworks -name '*.tbd' 2>/dev/null
    find usr/lib -name '*.tbd'
    echo SDKSettings.json
) >"$TBD_LIST"
log "  $(wc -l <"$TBD_LIST" | tr -d ' ') stubs"

remote_ssh --command="mkdir -p $REMOTE_SDK/usr/include $REMOTE_SDK/usr/lib" >/dev/null

# -L materializes symlinks: framework-top X.tbd files are symlinks through
# Versions/Current (itself a symlink), and neither intermediate link is in the
# file list — copied as symlinks they dangle on the VM and zig reports
# "unable to find framework".
log "Syncing .tbd stubs (symlinks dereferenced)..."
rsync -azL --files-from="$TBD_LIST" -e "$TRANSPORT" "$SDK/" "$REMOTE_HOST:$REMOTE_SDK/"
log "Syncing usr/include headers..."
rsync -azL -e "$TRANSPORT" "$SDK/usr/include/" "$REMOTE_HOST:$REMOTE_SDK/usr/include/"
log "Syncing libclang_rt.osx.a -> \$SDKROOT/usr/lib/ ..."
rsync -az -e "$TRANSPORT" "$CLANG_RT" "$REMOTE_HOST:$REMOTE_SDK/usr/lib/libclang_rt.osx.a"

# ---- remote provisioning ----------------------------------------------------
log "Uploading remote provisioning script..."
REMOTE_PROVISION=/tmp/spur-zigbuild-provision-remote.sh
PROVISION_LOCAL=$(mktemp)
cat >"$PROVISION_LOCAL" <<'REMOTE_EOF'
#!/usr/bin/env bash
# Remote half of scripts/zigbuild-provision-vm.sh. Idempotent.
# $1 (optional): the spur repo's rust-toolchain.toml channel — recorded in
# the bundle so startup-aws.sh can pre-install darwin targets for the pinned
# toolchain at boot.
set -euo pipefail
source /etc/profile.d/spur-build.sh
mkdir -p /mnt/cargo/zig-dist /mnt/cargo/tmp /mnt/cargo/macsdk
export TMPDIR=/mnt/cargo/tmp
RUST_PIN="${1:-}"
[ -n "$RUST_PIN" ] && printf '%s\n' "$RUST_PIN" > /mnt/cargo/macsdk/rust-pin.txt

echo "== rustup darwin targets =="
# Every toolchain: the repo pin (rust-toolchain.toml) resolves to its own
# toolchain inside ~/spur/main; adding only to the default leaves it without
# a darwin std (E0463 "can't find crate for core"). Both arches so
# `--target universal2-apple-darwin` (fat binary) works.
for tc in $(rustup toolchain list | awk '{print $1}'); do
    rustup target add --toolchain "$tc" aarch64-apple-darwin x86_64-apple-darwin
done

echo "== zig =="
if [ ! -x /mnt/cargo/zig/zig ]; then
    cd /mnt/cargo/zig-dist
    curl -fsSL https://ziglang.org/download/index.json -o index.json
    url=$(python3 - <<'PY'
import json
data = json.load(open('index.json'))
best = None
for ver, info in data.items():
    if ver == 'master' or '-' in ver:
        continue
    parts = tuple(int(x) for x in ver.split('.'))
    if parts >= (0, 16, 0):
        continue  # stay on a cargo-zigbuild-proven line
    t = info.get('aarch64-linux')
    if not t:
        continue
    if best is None or parts > best[0]:
        best = (parts, t['tarball'])
print(best[1])
PY
)
    echo "zig tarball: $url"
    curl -fsSL "$url" -o zig.tar.xz
    # zig tarballs extract to a dir named like the tarball; avoid tar -tf|head,
    # which SIGPIPEs tar under pipefail.
    base=$(basename "$url")
    dir="${base%.tar.xz}"
    tar -xf zig.tar.xz
    [ -x "/mnt/cargo/zig-dist/$dir/zig" ] || { echo "unexpected zig layout: $dir"; exit 1; }
    ln -sfn "/mnt/cargo/zig-dist/$dir" /mnt/cargo/zig
fi
ln -sf /mnt/cargo/zig/zig "$CARGO_HOME/bin/zig"
echo "zig version: $(/mnt/cargo/zig/zig version)"

echo "== cargo-zigbuild =="
command -v cargo-zigbuild >/dev/null 2>&1 || cargo install cargo-zigbuild --locked
echo "cargo-zigbuild: $(cargo-zigbuild --version)"

echo "== bindgen-cli (libproc bindings) =="
command -v bindgen >/dev/null 2>&1 || cargo install bindgen-cli --locked
bindgen --version

echo "== profile.d additions =="
if sudo -n true 2>/dev/null; then
    if ! grep -q 'zigbuild macOS cross' /etc/profile.d/spur-build.sh; then
        sudo tee -a /etc/profile.d/spur-build.sh >/dev/null <<'EOF'
# --- zigbuild macOS cross ---
export PATH="/mnt/cargo/zig:$PATH"
export SDKROOT=/mnt/cargo/macsdk/MacOSX.sdk
# cc-rs APPENDS these target-scoped flags AFTER the plain
# CFLAGS/-mcpu=native above, so the -mcpu here must be explicit: being last
# on the command line it overrides the neoverse-v2 (SVE) tuning that would
# otherwise poison darwin codegen. apple_m1 (zig cpu spelling) also
# sidesteps zig cc's inconsistent default feature mapping, which fails NEON
# code (blake3, zstd) with "always_inline function requires target feature
# 'altnzcv'". Every Apple Silicon Mac is >= M1.
export CFLAGS_aarch64_apple_darwin="-O2 -mcpu=apple_m1"
export CXXFLAGS_aarch64_apple_darwin="-O2 -mcpu=apple_m1"
EOF
        echo "profile.d updated"
    else
        echo "profile.d already updated"
    fi
    # Separate guard: VMs provisioned before universal2 support have the v1
    # block without the x86_64 flags.
    if ! grep -q 'CFLAGS_x86_64_apple_darwin' /etc/profile.d/spur-build.sh; then
        sudo tee -a /etc/profile.d/spur-build.sh >/dev/null <<'EOF'
# x86_64 slice of universal2 builds. The explicit -mcpu is NOT optional:
# modern cc-rs APPENDS the target-scoped flags after the plain CFLAGS above,
# so the profile's -mcpu=native (neoverse!) reaches the x86 compile line and
# must be overridden by a LATER -mcpu — clang/zig take the last one. The
# baseline x86_64 cpu keeps the slice runnable on every Intel Mac.
export CFLAGS_x86_64_apple_darwin="-O2 -mcpu=x86_64"
export CXXFLAGS_x86_64_apple_darwin="-O2 -mcpu=x86_64"
EOF
        echo "profile.d x86_64 flags added"
    fi
    # Self-heal libproc bindings on every dispatch shell (see startup-aws.sh
    # MACCROSS block — kept in lockstep): a fresh VM's first zigbuild fails
    # at libproc, the failed build leaves the OUT_DIR, and the next login
    # shell plants into it.
    if ! grep -q 'plant-libproc-bindings.sh' /etc/profile.d/spur-build.sh; then
        sudo tee -a /etc/profile.d/spur-build.sh >/dev/null <<'EOF'
# Self-heal libproc's cross-generated bindings (idempotent glob+cp, ~ms).
[ -x /mnt/cargo/macsdk/plant-libproc-bindings.sh ] && /mnt/cargo/macsdk/plant-libproc-bindings.sh >/dev/null 2>&1 || true
EOF
        echo "profile.d libproc self-heal added"
    fi
    # Keep ort-sys prebuilts on the cargo volume (lockstep with startup-aws.sh
    # MACCROSS / build.sh). Root reclaim used to wipe ~/.cache/ort.pyke.io and
    # race cargo xtask install with missing libonnxruntime.a.
    if ! grep -q 'XDG_CACHE_HOME=/mnt/cargo/.cache' /etc/profile.d/spur-build.sh; then
        sudo tee -a /etc/profile.d/spur-build.sh >/dev/null <<'EOF'
# ort-sys (pyke onnxruntime) + other XDG caches on the big disk — not root EBS.
mkdir -p /mnt/cargo/.cache
export XDG_CACHE_HOME=/mnt/cargo/.cache
EOF
        echo "profile.d XDG_CACHE_HOME added"
    fi
else
    echo "WARNING: no passwordless sudo; set SDKROOT/CFLAGS_aarch64_apple_darwin manually"
fi

echo "== ld64.lld link driver =="
# Final links go through clang + ld64.lld, not zig's Mach-O linker: with
# libc++ linked as a system dylib (see spur-cargo's zigbuild RUSTFLAGS),
# zig's linker fails with "relocation Overflow" on prebuilt Apple-clang
# objects (pyke onnxruntime). zig remains the C/C++ compiler. A MODERN lld
# is required — the same prebuilt objects reference objc_msgSend selector
# stubs (_objc_msgSend$sel, Xcode 14+), which lld-14 does not synthesize.
if sudo -n true 2>/dev/null; then
    command -v ld64.lld-19 >/dev/null 2>&1 \
        || sudo apt-get install -y -qq lld-19 >/dev/null 2>&1 \
        || sudo apt-get install -y -qq lld-16 >/dev/null 2>&1 || true
fi
cat > /mnt/cargo/macsdk/ld64-link.sh <<'LD64'
#!/usr/bin/env bash
# Final-link driver for spur-cargo zigbuild (macOS cross): clang + ld64.lld
# against the synced macOS SDK. zig remains the C/C++ *compiler*; its own
# Mach-O linker fails with "relocation Overflow" on prebuilt Apple-clang
# objects (pyke onnxruntime) once libc++ becomes a dylib import. Needs a
# modern ld64.lld: prebuilt Apple objects reference objc_msgSend selector
# stubs (_objc_msgSend$sel), unknown to lld-14.
SDK=/mnt/cargo/macsdk/MacOSX.sdk
LLD=$(command -v ld64.lld-19 || command -v ld64.lld-16 || command -v ld64.lld-14 || command -v ld64.lld)
# rustc always passes `-arch <arch>`; mirror it in clang's --target so one
# driver serves both slices of a universal2 build.
ARCH=arm64
prev=""
for a in "$@"; do
    if [ "$prev" = "-arch" ]; then ARCH="$a"; fi
    prev="$a"
done
# lance-linalg's AVX-512 dist_table kernel cannot be built by its own
# build.rs on a cross host (-march=native resolves to the arm host), yet its
# Rust caller is enabled whenever the avx512 cfg is set — provisioning
# pre-compiles the runtime-gated kernel and every x86_64 link picks it up
# here (dead-stripped wherever unreferenced).
if [ "$ARCH" = x86_64 ] && [ -f /mnt/cargo/macsdk/lance-dist-table-avx512.x86_64.o ]; then
    set -- "$@" /mnt/cargo/macsdk/lance-dist-table-avx512.x86_64.o
fi
# -mlinker-version=705: Debian clang assumes an ancient host ld64 and emits
# the legacy -macosx_version_min flag, which ld64.lld rejects with "must
# specify -platform_version"; claiming a modern linker switches clang to
# the -platform_version form.
exec clang --target="$ARCH-apple-macos11" --sysroot="$SDK" \
    -fuse-ld="$LLD" -mlinker-version=705 \
    -F"$SDK/System/Library/Frameworks" -L"$SDK/usr/lib" \
    "$@"
LD64
chmod +x /mnt/cargo/macsdk/ld64-link.sh
command -v clang >/dev/null || echo "WARNING: clang missing on VM (apt install clang lld)"
command -v ld64.lld-19 >/dev/null 2>&1 || command -v ld64.lld-16 >/dev/null 2>&1 \
    || echo "WARNING: no modern ld64.lld (apt install lld-19); final spur link will fail on objc stubs"

echo "== lance dist_table AVX-512 kernel (x86_64) =="
# See the driver comment above: lance-linalg's own build.rs uses
# -march=native for this kernel, which cannot work on a cross host, while
# the (feature-skipped) f16 kernel probes still set the avx512 cfg that
# enables the Rust caller. The kernel is runtime-gated behind AVX-512
# detection, so building it for the x86-64-v4 baseline is safe on every Mac.
LANCE_LINALG_SRC=$(ls -d "$CARGO_HOME"/registry/src/index.crates.io-*/lance-linalg-*/src/simd/dist_table.c 2>/dev/null | sort | tail -1 || true)
if [ -n "$LANCE_LINALG_SRC" ]; then
    /mnt/cargo/zig/zig cc -target x86_64-macos -std=c17 -O3 -funroll-loops \
        -mcpu=x86_64_v4 -c "$LANCE_LINALG_SRC" \
        -o /mnt/cargo/macsdk/lance-dist-table-avx512.x86_64.o
    echo "kernel compiled from $LANCE_LINALG_SRC"
else
    echo "WARNING: lance-linalg source not in registry yet (run a build first); x86_64 links will miss the avx512 dist_table kernel"
fi

echo "== libproc darwin bindings (stable copy + plant helper) =="
# libproc 0.14's build.rs gates bindgen behind #[cfg(target_os = "macos")] —
# a HOST cfg, so a Linux cross build runs the no-op linux main() and the lib
# then fails on the missing include. Pre-generate the bindings here; the
# plant helper copies them into whatever OUT_DIR hash the current flag set
# produced (the hash changes whenever RUSTFLAGS change).
SDK=/mnt/cargo/macsdk/MacOSX.sdk
printf '#include <libproc.h>\n' > /tmp/libproc_rs.h
for arch in aarch64 x86_64; do
    # rustfmt may be missing for the default toolchain; bindgen still writes
    # valid (single-line) bindings and exits 0, but retry once uncached just
    # in case the first pass raced an rustup component download.
    bindgen /tmp/libproc_rs.h \
        --rust-target 1.72 --rust-edition 2018 --no-layout-tests \
        -o "/mnt/cargo/macsdk/osx_libproc_bindings.$arch.rs" \
        -- -x c++ -target "$arch-apple-darwin" -isysroot "$SDK" -I"$SDK/usr/include" \
        2>/dev/null || bindgen /tmp/libproc_rs.h \
        --rust-target 1.72 --rust-edition 2018 --no-layout-tests \
        -o "/mnt/cargo/macsdk/osx_libproc_bindings.$arch.rs" \
        -- -x c++ -target "$arch-apple-darwin" -isysroot "$SDK" -I"$SDK/usr/include"
done
# Back-compat name used by earlier docs/sessions.
cp /mnt/cargo/macsdk/osx_libproc_bindings.aarch64.rs /mnt/cargo/macsdk/osx_libproc_bindings.rs

cat > /mnt/cargo/macsdk/plant-libproc-bindings.sh <<'PLANT'
#!/usr/bin/env bash
# Copy the pre-generated libproc darwin bindings into every libproc OUT_DIR
# under each darwin target (both universal2 slices). Run after a build fails
# with "couldn't read .../libproc-<hash>/out/osx_libproc_bindings.rs", then
# rebuild.
set -euo pipefail
planted=0
for arch in aarch64 x86_64; do
    for out in "$HOME"/spur/main/target/$arch-apple-darwin/*/build/libproc-*/out; do
        [ -d "$out" ] || continue
        cp "/mnt/cargo/macsdk/osx_libproc_bindings.$arch.rs" "$out/osx_libproc_bindings.rs"
        echo "planted: $out"
        planted=1
    done
done
[ "$planted" = 1 ] || echo "no libproc OUT_DIR found yet — run the build once first"
PLANT
chmod +x /mnt/cargo/macsdk/plant-libproc-bindings.sh
wc -c /mnt/cargo/macsdk/osx_libproc_bindings.*.rs
# GCP has a different disk lifecycle. AWS records this switch for stop/resume.
SPUR_AWS_EBS_TOOL_ROOT=0
if [[ -f /etc/default/spur-cargo-cache ]]; then
    source /etc/default/spur-cargo-cache
fi
# BEGIN persist macOS cross assets
# The cache is erased on stop/start. Keep a durable copy for the boot mount
# helper, including VMs provisioned after their initial cloud-init run.
if [[ "${SPUR_AWS_EBS_TOOL_ROOT:-1}" == "1" && -d /mnt/cargo/macsdk && -x /mnt/cargo/zig/zig ]]; then
    sudo mkdir -p /opt/spur-cross
    for asset in macsdk zig-dist; do
        if [[ ! /mnt/cargo/$asset -ef /opt/spur-cross/$asset ]]; then
            sudo mkdir -p "/opt/spur-cross/$asset"
            sudo cp -a "/mnt/cargo/$asset/." "/opt/spur-cross/$asset/"
        fi
    done
    # Resolve the executable, not just the link: after resume zig may point
    # through /opt/spur-cross/zig rather than directly at the version directory.
    zig_dir=$(dirname "$(readlink -f /mnt/cargo/zig/zig)")
    sudo ln -sfn "zig-dist/$(basename "$zig_dir")" /opt/spur-cross/zig
fi
# END persist macOS cross assets
echo "PROVISION_DONE"
REMOTE_EOF
rsync -az -e "$TRANSPORT" "$PROVISION_LOCAL" "$REMOTE_HOST:$REMOTE_PROVISION"
rm -f "$PROVISION_LOCAL"

# The repo's toolchain pin rides along so startup-aws.sh can pre-install
# darwin targets for it at boot (bundle restore path).
RUST_PIN="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$SPUR_REPO_ROOT/rust-toolchain.toml" 2>/dev/null | head -1)"

log "Running remote provisioning..."
remote_ssh --command="bash $REMOTE_PROVISION $(printf '%q' "$RUST_PIN")"

if [[ $PUBLISH_BUNDLE -eq 1 ]]; then
    BUNDLE_S3="s3://${SCCACHE_BUCKET}/macsdk/macos-cross-bundle.tar.gz"
    log "Publishing macOS cross bundle to $BUNDLE_S3 ..."
    remote_ssh --command="bash -lc 'set -euo pipefail
        cd /mnt/cargo
        zigdir=zig-dist/\$(basename \"\$(dirname \"\$(readlink -f /mnt/cargo/zig/zig)\")\")
        packdir=\$(mktemp -d /mnt/cargo/tmp/macos-cross-pack.XXXXXX)
        cleanup_pack() { rm -rf \"\$packdir\"; }
        trap cleanup_pack EXIT
        # Materialize EBS-backed directories; preserve a portable Zig symlink.
        tar -chf \"\$packdir/bundle.tar\" \\
            macsdk \"\$zigdir\" \\
            cargo-home/bin/cargo-zigbuild cargo-home/bin/bindgen
        ln -s \"\$zigdir\" \"\$packdir/zig\"
        tar -rf \"\$packdir/bundle.tar\" -C \"\$packdir\" zig
        gzip \"\$packdir/bundle.tar\"
        aws s3 cp --region ${SCCACHE_S3_REGION} \\
            \"\$packdir/bundle.tar.gz\" \"$BUNDLE_S3\"'"
    log "Bundle published; fresh VMs restore it at boot via startup-aws.sh."
fi

log "Done. Build with: scripts/spur-cargo zigbuild --release -p spur-cli"
log "(add --target universal2-apple-darwin for a fat arm64+x86_64 binary)"
log "Darwin builds check libproc bindings before Cargo and supply them before rustc."
