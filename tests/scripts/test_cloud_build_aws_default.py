import os
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CLOUD_BUILD = ROOT / "scripts" / "cloud-build"
SPUR_CARGO = ROOT / "scripts" / "spur-cargo"


def test_cloud_build_is_centralized_as_notebook_symlink():
    assert CLOUD_BUILD.is_symlink()

    target = os.readlink(CLOUD_BUILD)
    assert target == "../../spur-notebook/scripts/cloud-build"


def test_spur_does_not_vendor_cloud_build_provider_scripts():
    assert not CLOUD_BUILD.resolve().is_relative_to(ROOT)


def test_spur_cargo_routes_remote_builds_through_centralized_cloud_build():
    script = SPUR_CARGO.read_text()

    assert "resolve_remote_build_sh()" in script
    assert 'candidate="$SCRIPT_DIR/cloud-build/build.sh"' in script
    assert 'REMOTE_BUILD_SH="$(resolve_remote_build_sh || true)"' in script
    assert "LEGACY_REMOTE_BUILD_SH" not in script
    assert 'PRIMARY_CLOUD="${SPUR_CLOUD:-aws-my}"' in script
    assert 'FALLBACK_CLOUD="${SPUR_CLOUD_FALLBACK-aws}"' in script
    assert 'SPUR_REMOTE_NAMESPACE="${SPUR_REMOTE_NAMESPACE:-spur}" SPUR_CLOUD="$cloud" "$REMOTE_BUILD_SH" --auto-spin -- "$@"' in script
    assert "remote $cloud VM unavailable" in script
    assert "remote cargo exited $REMOTE_EXIT" in script


def test_centralized_cloud_build_stages_prune_helper_for_symlinked_consumers():
    script = (CLOUD_BUILD / "build.sh").read_text()

    assert 'remote_prune_helper="/mnt/cargo/cargo-home/spur-prune-remote.$WORKTREE_FILE_KEY.sh"' in script
    assert '"$SCRIPT_DIR/_prune-remote.sh" "$REMOTE_HOST:$remote_prune_helper"' in script
    assert '--command="bash \\"$remote_prune_helper\\"' in script


def test_centralized_cloud_build_defaults_namespace_from_calling_repo():
    for script_name in ["build.sh", "fetch.sh"]:
        script = (CLOUD_BUILD / script_name).read_text()

        assert 'DEFAULT_REMOTE_NAMESPACE=$(basename "$GIT_TOPLEVEL")' in script
        assert 'REMOTE_NAMESPACE="${SPUR_REMOTE_NAMESPACE:-$DEFAULT_REMOTE_NAMESPACE}"' in script
