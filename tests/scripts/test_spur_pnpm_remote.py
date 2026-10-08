import os
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPUR_PNPM = ROOT / "scripts" / "spur-pnpm"
LINT_INVARIANTS = ROOT / ".github" / "workflows" / "lint-invariants.yml"
BUILD_README = ROOT / "scripts" / "cloud-build" / "README.md"


def run_spur_pnpm(args, env=None):
    run_env = os.environ.copy()
    run_env.pop("SPUR_NOTEBOOK_REPO", None)
    run_env.update(env or {})

    return subprocess.run(
        [str(SPUR_PNPM), *args],
        cwd=ROOT,
        env=run_env,
        text=True,
        capture_output=True,
        check=False,
    )



def test_lint_invariants_private_notebook_checkout_uses_explicit_secret():
    workflow = LINT_INVARIANTS.read_text()

    assert "repository: getspur/spur-notebook" in workflow
    assert "SPUR_NOTEBOOK_CHECKOUT_TOKEN" in workflow
    assert "token: ${{ secrets.SPUR_NOTEBOOK_CHECKOUT_TOKEN }}" in workflow
    assert "Missing SPUR_NOTEBOOK_CHECKOUT_TOKEN" in workflow

    docs = BUILD_README.read_text()
    docs_single_line = " ".join(docs.split())
    assert "SPUR_NOTEBOOK_CHECKOUT_TOKEN" in docs
    assert (
        "read access to the private `getspur/spur-notebook` repository"
        in docs_single_line
    )


def test_spur_pnpm_without_repo_prints_post_split_guidance():
    result = run_spur_pnpm(["test", "--", "src/ui/notebook/NotebookCells.test.tsx"])

    assert result.returncode == 2
    assert "Notebook frontend commands now live in getspur/spur-notebook" in result.stderr
    assert "SPUR_NOTEBOOK_REPO=/path/to/spur-notebook" in result.stderr


def test_spur_pnpm_forwards_to_standalone_jute_notebook(tmp_path):
    notebook_repo = tmp_path / "spur-notebook"
    frontend_dir = notebook_repo / "jute-notebook"
    frontend_dir.mkdir(parents=True)

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    capture = tmp_path / "pnpm-args.txt"
    fake_pnpm = bin_dir / "pnpm"
    fake_pnpm.write_text(
        "#!/usr/bin/env bash\n"
        "printf '%s\\n' \"$@\" >\"$PNPM_CAPTURE\"\n"
    )
    fake_pnpm.chmod(0o755)

    result = run_spur_pnpm(
        ["--", "test", "--", "src/ui/notebook/NotebookCells.test.tsx"],
        env={
            "PATH": f"{bin_dir}{os.pathsep}{os.environ.get('PATH', '')}",
            "PNPM_CAPTURE": str(capture),
            "SPUR_NOTEBOOK_REPO": str(notebook_repo),
        },
    )

    assert result.returncode == 0
    assert capture.read_text().splitlines() == [
        "--dir",
        str(frontend_dir),
        "test",
        "--",
        "src/ui/notebook/NotebookCells.test.tsx",
    ]
