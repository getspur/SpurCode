"""Exercise the supported build providers and CI credential wiring offline."""
import json
import os
from pathlib import Path
import shutil
import subprocess

import pytest


ROOT = Path(__file__).resolve().parents[2]
CLOUD = ROOT / "scripts/cloud-build"
BUCKET = "spurlab-591950085580-spur-sccache-apse5"


def test_removed_provider_is_rejected_before_any_cloud_call():
    result = subprocess.run(
        ["bash", "-ec", 'source "$1/config.env"', "bash", str(CLOUD)],
        env=dict(os.environ, SPUR_CLOUD="gcp"), capture_output=True, text=True,
    )
    assert result.returncode == 2
    assert "expected: aws | aws-my" in result.stderr


def test_clean_checkout_uses_current_account_cache(tmp_path):
    for name in ("config.env", "config-aws-my.env"):
        shutil.copy2(CLOUD / name, tmp_path / name)
    result = subprocess.run(
        ["bash", "-ec", 'source "$1/config.env"; printf "%s" "$SCCACHE_BUCKET"', "bash", str(tmp_path)],
        env={"PATH": os.environ["PATH"], "HOME": str(tmp_path), "SPUR_CLOUD": "aws-my"},
        capture_output=True, text=True, check=True,
    )
    assert result.stdout == BUCKET


def test_ci_uses_aws_oidc_and_current_account_cache():
    workflow = (ROOT / ".github/workflows/ci.yml").read_text()
    for name in ("check-test", "clippy"):
        job = workflow.split(f"\n  {name}:\n", 1)[1].split("\n  #", 1)[0]
        assert "SCCACHE_BUCKET: ${{ vars.AWS_SCCACHE_BUCKET }}" in job
        assert "aws-actions/configure-aws-credentials@" in job
        assert "role-to-assume: ${{ vars.AWS_SCCACHE_ROLE_ARN }}" in job
        assert "if: github.event_name != 'pull_request'" in job
        assert 'allowed-account-ids: "591950085580"' in job
    for path in (ROOT / ".github/workflows").glob("*.yml"):
        text = path.read_text()
        assert "google-github-actions/auth" not in text, path
        assert "wiilearn-spur-sccache" not in text, path


@pytest.mark.parametrize("repo", [ROOT, ROOT.parent / "spur-notebook"])
def test_stale_google_cache_settings_cannot_select_gcs(repo, tmp_path):
    fake = tmp_path / "sccache"
    fake.write_text('#!/usr/bin/env python3\nimport json, os\nprint(json.dumps({k:v for k,v in os.environ.items() if k.startswith("SCCACHE_")}))\n')
    fake.chmod(0o755)
    env = dict(os.environ, PATH=str(tmp_path) + os.pathsep + os.environ["PATH"],
               SPUR_SCCACHE_GCS="1", SCCACHE_GCS_BUCKET="retired-cache",
               SCCACHE_MULTILEVEL_CHAIN="disk,gcs")
    for key in ("SPUR_SCCACHE_S3", "SCCACHE_BUCKET", "SCCACHE_REGION"):
        env.pop(key, None)
    result = subprocess.run([str(repo / "scripts/sccache-worktree.sh"), "--show-stats"],
                            cwd=repo, env=env, capture_output=True, text=True, check=True)
    cache = json.loads(result.stdout)
    assert cache["SCCACHE_BUCKET"] == BUCKET
    assert cache["SCCACHE_MULTILEVEL_CHAIN"] == "disk,s3"
    assert "SCCACHE_GCS_BUCKET" not in cache


def test_no_legacy_gcp_builder_remains():
    assert not (ROOT / "scripts/gcp-build").exists()
    assert not (CLOUD / "provider-gcp.sh").exists()
