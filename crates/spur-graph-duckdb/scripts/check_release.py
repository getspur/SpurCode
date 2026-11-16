"""Check an unsigned local release; never publish, install, or reuse old outputs.

    python scripts/check_release.py --out /fresh/task/output --host-native

Run from the repository or use this script's absolute path. Python >=3.11 and
requirements-test.txt (DuckDB exactly 2.0.0) must already be installed. Rust is
the existing repository pin. Rust unit tests use spur-cargo's remote default
with local fallback disabled. --host-native explicitly authorizes a separate
host-target build: the local DuckDB process must LOAD that target's library and
execute its native oracles; a remote Linux library cannot load on macOS.
Hosted CI uses --rust-tests-mode host for the same declared native target,
without project VM or publication credentials. This is never a retry of a
failed remote command. Every nonzero phase stops the run with its raw exit.

receipt.json, command stdout/stderr, source inputs, native files, the verified
package/sidecar and unittest summary remain in the exclusively created --out.
An existing output directory is an error. Configuring CI is not evidence that
its runner has executed. Only a completed receipt records observed readiness.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import tomllib
import unittest


CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]
WRAPPER = ROOT / "scripts/spur-cargo"
HISTORICAL_SHA256 = "e7d191bcda2e0e8ea52d249189007e1a7e424f1b2b8fb987dfebde037a575028"
EXAMPLES = ("catalog_git_sources", "history_catalog", "reference_file_facts",
            "reference_symbols", "retrieval_native", "temporal_reference",
            "temporal_symbol_oracle")


def checksum(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def release_trust():
    return dict(manifest=os.environ.get("SPUR_RELEASE_MANIFEST"),
                manifest_sha256=os.environ.get("SPUR_RELEASE_MANIFEST_SHA256"),
                source_identity=os.environ.get("SPUR_RELEASE_SOURCE_IDENTITY"))


def approved_extension_sha(extension, *, manifest=None, manifest_sha256=None, source_identity=None):
    """Historical approval OR explicit caller trust in a verified current manifest.

    The caller must obtain the manifest digest after verification, and the
    source identity independently from the fresh build's source receipt. Never
    compute a new approval from an arbitrary extension's runtime hash alone.
    This is unsigned local trust, not publisher authentication.
    """
    extension = Path(extension).resolve(strict=True)
    if not any((manifest, manifest_sha256, source_identity)):
        if checksum(extension) != HISTORICAL_SHA256:
            raise ValueError("unapproved extension hash; explicit verified release manifest required")
        return HISTORICAL_SHA256
    if not all((manifest, manifest_sha256, source_identity)):
        raise ValueError("release manifest, manifest checksum and independent source identity are all required")
    manifest = Path(manifest).resolve(strict=True)
    if checksum(manifest) != manifest_sha256:
        raise ValueError("release manifest checksum mismatch")
    if manifest != Path(str(extension) + ".json").resolve():
        raise ValueError("release manifest does not belong to this artifact")
    sys.path.insert(0, str(CRATE / "scripts"))
    from verify_package import verify_package
    verified = verify_package(extension)
    if verified["source_identity"] != source_identity:
        raise ValueError("release source identity mismatch")
    if checksum(manifest) != manifest_sha256:
        raise ValueError("release manifest changed during verification")
    return verified["binary"]["sha256"]


def run_phases(phases, output, env, receipt):
    """Real child execution seam; record raw exits, stop at the first failure."""
    for name, command in phases:
        stdout, stderr = Path(output) / (name + ".stdout"), Path(output) / (name + ".stderr")
        record = dict(phase=name, argv=list(map(str, command)), cwd=str(ROOT),
                      stdout=str(stdout), stderr=str(stderr))
        receipt["commands"].append(record)
        print(json.dumps({"phase": name, "argv": record["argv"]}), flush=True)
        with stdout.open("xb") as out, stderr.open("xb") as err:
            try:
                process = subprocess.run(command, cwd=ROOT, env=env, stdout=out, stderr=err)
            except OSError as error:
                record["launch_error"] = str(error)
                record["returncode"] = None
                raise
        record["returncode"] = process.returncode
        print(json.dumps({"phase": name, "raw_exit": process.returncode,
                          "stdout": str(stdout), "stderr": str(stderr)}), flush=True)
        if process.returncode:
            return process.returncode
    return 0


def run_unittests(directory, summary):
    suite = unittest.defaultTestLoader.discover(str(directory), pattern="test_*.py")
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    report = dict(tests_run=result.testsRun, failures=len(result.failures), errors=len(result.errors),
                  skipped=len(result.skipped), expected_failures=len(result.expectedFailures),
                  unexpected_successes=len(result.unexpectedSuccesses),
                  skipped_tests=[dict(test=str(test), reason=reason) for test, reason in result.skipped])
    passed = (result.wasSuccessful() and result.testsRun > 0
              and not result.skipped and not result.expectedFailures)
    report["release_pass"] = passed
    Path(summary).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return 0 if passed else 1


def runtime_preflight():
    try:
        import duckdb
    except ImportError as error:
        raise ValueError("install requirements-test.txt: DuckDB exactly 2.0.0 is required") from error
    if duckdb.__version__ != "2.0.0":
        raise ValueError(f"DuckDB exactly 2.0.0 is required; got {duckdb.__version__}")
    import _duckdb
    with duckdb.connect() as connection:
        version = connection.execute("SELECT version()").fetchone()[0]
        target = connection.execute("PRAGMA platform").fetchone()[0]
    if version != "v2.0.0":
        raise ValueError(f"native DuckDB exactly 2.0.0 is required; got {version}")
    return dict(duckdb=duckdb.__version__, native_version=version, duckdb_platform=target,
                python=sys.version, python_executable=sys.executable,
                python_sha256=checksum(sys.executable), native_engine=_duckdb.__file__,
                native_engine_sha256=checksum(_duckdb.__file__),
                host=platform.platform(), machine=platform.machine())


def native_environment(artifacts, output):
    required = ("library", "build_script", *EXAMPLES)
    for name in required:
        path = Path(artifacts.get(name, "/missing-native-oracle"))
        if not path.is_file() or not path.resolve().is_relative_to(Path(output).resolve()):
            raise ValueError(f"missing fresh native build/oracle {name}: {path}")
        if name != "library" and not os.access(path, os.X_OK):
            raise ValueError(f"native oracle is not executable: {name}")
    examples = Path(artifacts["history_catalog"]).parent
    if any(Path(artifacts[name]).parent != examples for name in EXAMPLES):
        raise ValueError("native oracles must share this build's examples directory")
    return dict(SPUR_GRAPH_LIBRARY=str(artifacts["library"]),
                SPUR_GRAPH_BUILD_SCRIPT=str(artifacts["build_script"]),
                SPUR_FILE_FACTS_ORACLE=str(artifacts["reference_file_facts"]),
                SPUR_RETRIEVAL_NATIVE=str(artifacts["retrieval_native"]),
                SPUR_HISTORY_CATALOG=str(artifacts["history_catalog"]),
                SPUR_TEMPORAL_EXAMPLES_DIR=str(examples))


def build_artifacts(log):
    artifacts = {}
    for line in Path(log).read_text().splitlines():
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        target = message["target"]
        if Path(target["src_path"]).resolve().is_relative_to(CRATE):
            if "example" in target["kind"] and message.get("executable"):
                artifacts[target["name"]] = message["executable"]
            elif "custom-build" in target["kind"]:
                artifacts["build_script"] = message["filenames"][0]
            elif "cdylib" in target["kind"]:
                artifacts["library"] = message["filenames"][0]
    return artifacts


def verify_source(receipt):
    digest = hashlib.sha256(b"spur-graph-extractor-source-v1\0")
    paths = [item["path"] for item in receipt["inputs"]]
    if paths != sorted(set(paths)):
        raise ValueError("source receipt paths are not unique and sorted")
    for item in receipt["inputs"]:
        label = item["path"].encode()
        data = item["content"].encode() if "content" in item else (ROOT / item["path"]).read_bytes()
        if hashlib.sha256(data).hexdigest() != item["sha256"]:
            raise ValueError("working source differs from build input: " + item["path"])
        digest.update(len(label).to_bytes(8, "little"))
        digest.update(label)
        digest.update(len(data).to_bytes(8, "little"))
        digest.update(data)
    if receipt["extractor_id"] != "source-sha256-v1:" + digest.hexdigest():
        raise ValueError("source receipt identity mismatch")
    return receipt["extractor_id"]


def rust_test_arguments(output, mode):
    # A host absolute output path is not a writable path on the Linux VM.
    target = (Path("target/release-check") / Path(output).name / "rust-tests"
              if mode == "remote" else Path(output) / "rust-test-target")
    return ["test", "--locked", "--lib", "--target-dir", str(target)]


def check_release(args, output, receipt):
    receipt["runtime"] = runtime_preflight()
    if args.expect_platform and receipt["runtime"]["duckdb_platform"] != args.expect_platform:
        raise ValueError("observed DuckDB platform differs from configured target")
    if not args.host_native:
        raise ValueError("--host-native required: native packaging and oracles run on this host's target")
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", SPUR_NO_LOCAL_FALLBACK="1")

    def phase(name, command, overrides=None):
        code = run_phases([(name, command)], output, dict(env, **(overrides or {})), receipt)
        if code:
            raise ChildProcessError(code)

    phase("source-head", ["git", "rev-parse", "HEAD"])
    phase("source-status", ["git", "status", "--porcelain=v1"])
    receipt["source_head"] = (output / "source-head.stdout").read_text().strip()
    receipt["source_status"] = (output / "source-status.stdout").read_text()
    phase("rustc-version", ["rustc", "-vV"])
    rustc = dict(line.split(": ", 1) for line in (output / "rustc-version.stdout").read_text().splitlines() if ": " in line)
    pin = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    if rustc["release"] != pin:
        raise ValueError(f"Rust runtime must match repository pin {pin}; got {rustc['release']}")
    receipt["rustc"] = rustc
    receipt["configured_target"] = rustc["host"]
    phase("cargo-version", [str(WRAPPER), "--version"])
    receipt["cargo_version"] = (output / "cargo-version.stdout").read_text().strip()
    native_target = output / "native-target"
    rust_args = [str(WRAPPER), "--workdir", str(CRATE)]
    rust_test_env = {"SPUR_REMOTE": "0"} if args.rust_tests_mode == "host" else {"SPUR_REMOTE": "1"}
    receipt["rust_tests_mode"] = args.rust_tests_mode
    phase("rust-tests", [*rust_args, *rust_test_arguments(output, args.rust_tests_mode)], rust_test_env)
    phase("native-build", [*rust_args, "build", "--locked", "--release", "--lib", "--examples",
                           "--target", rustc["host"], "--target-dir", str(native_target),
                           "--message-format=json"], {"SPUR_REMOTE": "0"})
    artifacts = build_artifacts(output / "native-build.stdout")
    env.update(native_environment(artifacts, output))
    receipt["native_artifacts"] = {name: dict(path=path, sha256=checksum(path)) for name, path in artifacts.items()}
    probe = output / "source-probe"
    probe.mkdir()
    phase("source-probe", [artifacts["build_script"]],
          dict(CARGO_MANIFEST_DIR=str(CRATE), OUT_DIR=str(probe)))
    source_receipt = json.loads((probe / "spur_graph_source_identity.json").read_text())
    source_id = verify_source(source_receipt)
    extension = output / "spur_graph.duckdb_extension"
    phase("package", [sys.executable, str(CRATE / "scripts/package_extension.py"),
                      "--library", artifacts["library"], "--out", str(extension)])
    phase("verify-package", [sys.executable, str(CRATE / "scripts/verify_package.py"), str(extension)])
    verified = json.loads((output / "verify-package.stdout").read_text())["artifact"]
    manifest = Path(str(extension) + ".json").resolve(strict=True)
    trust = dict(manifest=str(manifest), manifest_sha256=checksum(manifest), source_identity=source_id)
    approved_extension_sha(extension, **trust)
    receipt["package"] = verified
    receipt["release_trust"] = trust
    env.update(SPUR_GRAPH_EXTENSION=str(extension), SPUR_RELEASE_MANIFEST=trust["manifest"],
               SPUR_RELEASE_MANIFEST_SHA256=trust["manifest_sha256"], SPUR_RELEASE_SOURCE_IDENTITY=source_id)
    receipt["test_environment"] = {key: value for key, value in env.items() if key.startswith("SPUR_") and key in {
        *native_environment(artifacts, output), "SPUR_GRAPH_EXTENSION", "SPUR_RELEASE_MANIFEST",
        "SPUR_RELEASE_MANIFEST_SHA256", "SPUR_RELEASE_SOURCE_IDENTITY"}}
    phase("unittests", [sys.executable, str(Path(__file__).resolve()), "--unittest-child",
                        str(CRATE / "tests"), "--summary", str(output / "unittest-summary.json")])
    # Re-enumerate current inputs and inherited settings using this build's probe.
    final_probe = output / "source-probe-final"
    final_probe.mkdir()
    phase("source-probe-final", [artifacts["build_script"]],
          dict(CARGO_MANIFEST_DIR=str(CRATE), OUT_DIR=str(final_probe)))
    final_source_receipt = json.loads((final_probe / "spur_graph_source_identity.json").read_text())
    if verify_source(final_source_receipt) != source_id:
        raise ValueError("source changed during release checks")
    approved_extension_sha(extension, **trust)
    if any(checksum(item["path"]) != item["sha256"] for item in receipt["native_artifacts"].values()):
        raise ValueError("native build artifacts changed during release checks")
    receipt["unittests"] = json.loads((output / "unittest-summary.json").read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--host-native", action="store_true")
    parser.add_argument("--rust-tests-mode", choices=("remote", "host"), default="remote")
    parser.add_argument("--expect-platform")
    parser.add_argument("--unittest-child", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--summary", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.unittest_child:
        if not args.summary:
            parser.error("unittest child requires --summary")
        return run_unittests(args.unittest_child, args.summary)
    if not args.out:
        parser.error("--out is required")
    output = args.out.absolute()
    output.mkdir(parents=True)  # Exclusive: no stale target, receipt or artifact adoption.
    receipt = dict(status="running", commands=[], driver_sha256=checksum(__file__))
    code = 0
    try:
        check_release(args, output, receipt)
        receipt["status"] = "passed"
    except ChildProcessError as error:
        code = error.args[0]
        receipt.update(status="failed", raw_child_exit=code)
        print(f"Release check failed: child exit {code}; see {output / 'receipt.json'}", file=sys.stderr)
    except Exception as error:
        code = 1
        receipt.update(status="failed", error=f"{type(error).__name__}: {error}")
        print("Release check failed: " + receipt["error"], file=sys.stderr)
    finally:
        receipt["exit_code"] = code
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    if code < 0:
        # Preserve a signal's real process outcome as well as its negative raw receipt.
        signal.signal(-code, signal.SIG_DFL)
        os.kill(os.getpid(), -code)
    return code


if __name__ == "__main__":
    sys.exit(main())
