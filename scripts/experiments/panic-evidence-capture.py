#!/usr/bin/env python3
"""Bounded, standard-library-only CGA capture. No install or graph writes.

The pinned driver runs as RUSTC through spur-cargo. Every emitted compiler JSON
fragment is retained verbatim; the bin fragment (containing ::main) is imported
verbatim. cargo-cga is hash-bound producer lineage, not used as a launcher: its
implicit build-std/cargo invocation would escape this bounded command contract.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

REVISION = "2fc63108f0b674fe7a178191ffa15647a18879f1"
TOOLCHAIN = "nightly-2026-10-18"
RUSTC_COMMIT = "8006091eb71ae7d9e9bccb66c6122c9d68af4466"
ALLOWLIST = [
    "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_BUILD_TARGET", "CARGO_INCREMENTAL",
    "CARGO_PROFILE_DEV_OPT_LEVEL", "CARGO_PROFILE_DEV_DEBUG_ASSERTIONS",
    "CARGO_PROFILE_DEV_OVERFLOW_CHECKS", "CARGO_PROFILE_DEV_PANIC",
]
REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "crates/spur-graph/tests/fixtures/panic_evidence/corpus.json"
# This exact absolute wrapper is required even for out-of-tree fixtures.
WRAPPER = Path("/Users/kevintruong/project/SpurCode/scripts/spur-cargo")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def input_kind(path):
    if path.endswith(".rs"):
        return "source"
    if path.endswith("Cargo.toml"):
        return "manifest"
    if path.endswith("Cargo.lock"):
        return "lockfile"
    return "config"


def inventory(root):
    rows = []
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), "symlink in closed fixture inventory")
        if path.is_file():
            relative = path.relative_to(root).as_posix()
            rows.append({"path": relative, "kind": input_kind(relative),
                         "sha256": digest(path.read_bytes())})
    return rows


def inventory_hash(rows):
    return digest(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode())


def flags(config):
    return ["-Zalways-encode-mir", "-Zhint-mostly-unused", "-Copt-level=0",
            "-Cpanic=unwind",
            "-Cdebug-assertions=" + ("on" if config["debug_assertions"] else "off"),
            "-Coverflow-checks=" + ("on" if config["overflow_checks"] else "off")]


def prepare(work):
    require(not work.is_relative_to(REPO), "capture must be outside the workspace")
    work.mkdir(parents=True, exist_ok=False)
    fixture = work / "fixture"
    fixture.mkdir()
    corpus = json.loads(CORPUS.read_text())
    for relative, text in corpus["files"].items():
        path = Path(relative)
        require(not path.is_absolute() and relative == path.as_posix()
                and all(p not in (".", "..", "") for p in relative.split("/")),
                "noncanonical corpus path")
        destination = fixture / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(text.encode())
    # Cargo discovers configuration in ancestors independently of CARGO_HOME.
    for parent in fixture.parents:
        for name in ("config", "config.toml"):
            require(not (parent / ".cargo" / name).exists(),
                    "ambient ancestor Cargo configuration is unsupported")
    configurations = [dict(c, compiler_flags=flags(c)) for c in corpus["configurations"]]
    plan = {"fixture": str(fixture), "inputs": inventory(fixture),
            "configurations": configurations, "wrapper": str(WRAPPER),
            "corpus_sha256": digest(CORPUS.read_bytes())}
    write_json(work / "plan.json", plan)
    return fixture, plan


def closed_environment(work):
    # Read shared rustup toolchains; isolate Cargo home/config and all outputs.
    rustup_home = Path(os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))).resolve()
    cargo_proxy = Path(shutil.which("rustup") or "").resolve().parent
    require((cargo_proxy / "cargo").is_file(), "rustup cargo proxy unavailable")
    cargo_home = work / "cargo-home"
    cargo_home.mkdir()
    return {"PATH": os.pathsep.join([str(cargo_proxy), "/usr/bin", "/bin", "/usr/sbin", "/sbin"]),
            "RUSTUP_HOME": str(rustup_home), "CARGO_HOME": str(cargo_home),
            "RUSTUP_TOOLCHAIN": TOOLCHAIN, "SPUR_REMOTE": "0",
            "RUSTC_WRAPPER": "", "CARGO_INCREMENTAL": "0",
            "LC_ALL": "C", "TMPDIR": str(work)}


def run(argv, cwd, env, directory, name, stdin=None, expected_success=True):
    directory.mkdir(parents=True, exist_ok=True)
    start = time.time()
    result = subprocess.run([str(a) for a in argv], cwd=cwd, env=env, input=stdin,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    stdout = directory / (name + ".stdout")
    stderr = directory / (name + ".stderr")
    stdout.write_bytes(result.stdout)
    stderr.write_bytes(result.stderr)
    record = {"argv": [str(a) for a in argv], "cwd": str(cwd), "environment": env,
              "exit": result.returncode, "elapsed_seconds": time.time() - start,
              "stdin_sha256": None if stdin is None else digest(stdin),
              "stdout": str(stdout), "stdout_sha256": digest(result.stdout),
              "stderr": str(stderr), "stderr_sha256": digest(result.stderr)}
    write_json(directory / (name + ".command.json"), record)
    if expected_success:
        require(result.returncode == 0, name + " failed; see " + str(stderr))
    return result


def example(binary, request, work, env, directory, name, success=True):
    # Invoking this prebuilt example does not compile or invoke cargo.
    data = json.dumps(request, ensure_ascii=False).encode()
    (directory / (name + ".request.json")).write_bytes(data)
    result = run([binary], work, env, directory, name, data, success)
    if success:
        return json.loads(result.stdout)
    require(result.returncode != 0, "negative control accepted: " + name)
    return {"rejected": True}


def producer(args, work):
    driver, launcher, source = (Path(v).resolve(strict=True) for v in
                                (args.cga_driver, args.cargo_cga, args.cga_source))
    require(digest(driver.read_bytes()) == args.expected_driver_sha256, "driver pin mismatch")
    require(digest(launcher.read_bytes()) == args.expected_launcher_sha256, "launcher pin mismatch")
    head = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"]).decode().strip()
    require(head == REVISION, "CGA source revision mismatch")
    patch = subprocess.check_output(["git", "-C", str(source), "diff", "--binary", "HEAD"])
    if args.producer_patch:
        declared = Path(args.producer_patch).read_bytes()
        require(patch == declared and bool(patch), "producer patch differs from source checkout")
        (work / "producer.patch").write_bytes(patch)
        build = {"state": "patched", "patch_sha256": digest(patch)}
    else:
        require(not patch, "modified producer requires retained --producer-patch")
        build = {"state": "unmodified"}
    require(not subprocess.check_output(["git", "-C", str(source), "ls-files", "--others", "--exclude-standard"]),
            "untracked producer sources are unsupported")
    return driver, {"build": build, "cargo_cga_sha256": digest(launcher.read_bytes()),
                    "cga_driver_sha256": digest(driver.read_bytes())}, launcher


def scalar_range(source, span):
    lines = source.splitlines(keepends=True)
    def offset(position):
        line, column = position["line"], position["column"]
        require(0 < line <= len(lines), "invalid compiler line")
        text = lines[line - 1].rstrip("\r\n")
        require(0 < column <= len(text) + 1, "invalid compiler scalar column")
        return len(("".join(lines[:line - 1]) + text[:column - 1]).encode())
    return [offset(span["start"]), offset(span["end"])]


def acceptance(evidence, sites, fixture, config):
    observations = evidence["observations"]
    source = (fixture / "src/lib.rs").read_text()
    wanted = 'x.expect("unicode")'
    start = source.encode().index(wanted.encode())
    expected = [start, start + len(wanted.encode())]
    unicode_rows = []
    for obs in observations:
        loc = obs["raw_location"]
        if obs["converted_location"]["file_path"] == "src/lib.rs" and "::unicode" in obs["canonical_caller"]:
            converted = scalar_range(source, loc["span"])
            require(converted == obs["converted_location"]["byte_range"], "independent compiler coordinate conversion mismatch")
            if converted == expected:
                unicode_rows.append(obs)
    require(unicode_rows, "compiler did not retain required Unicode call coordinate witness")
    # This is a byte-level endpoint check against an independently located call,
    # not a convention inferred from the exporter RangeInclusive type.
    witness = unicode_rows[0]
    require(source.encode()[expected[0]:expected[1]].decode() == wanted, "Unicode call byte witness mismatch")
    same_line_prefix = source.encode()[:start].decode().rsplit("\n", 1)[-1]
    require(len(same_line_prefix.encode()) > len(same_line_prefix), "Unicode witness needs a multibyte prefix")
    require(witness["raw_location"]["span"]["start"]["column"] == len(same_line_prefix) + 1,
            "actual compiler columns are not one-based scalar columns")
    require(evidence["capabilities"] == {"receiver_type": "unavailable", "assert_reason": "unavailable", "macro_expansion": "unavailable"},
            "unavailable compiler capability must remain explicit")
    # Keep all actual observations for each held-out caller, including unmapped
    # and optimizer-elided source sites. Names here select corpus callers only;
    # they never decide an association or infer a receiver type.
    cases = {}
    for name in ["options", "results", "custom", "chain", "generic", "unicode", "expanded",
                 "dependency", "feature_control", "indexing", "addition", "debug_control"]:
        rows = [o for o in observations if "::" + name in o["canonical_caller"]]
        cases[name] = {"observations": rows, "count": len(rows)}
    for name, count in {"options": 4, "results": 4, "custom": 2, "chain": 2,
                        "generic": 2, "unicode": 1, "expanded": 1,
                        "dependency": 1, "indexing": 1}.items():
        require(cases[name]["count"] == count, "required held-out compiler observations changed: " + name)
    require(cases["addition"]["count"] == int(config["overflow_checks"]), "overflow compiler contrast mismatch")
    require(cases["debug_control"]["count"] == int(config["debug_assertions"]), "debug compiler contrast mismatch")
    require(cases["feature_control"]["count"] == int(bool(config["features"])), "feature compiler contrast mismatch")
    require(cases["expanded"]["observations"][0]["association"]["state"] == "unmapped", "macro expansion must remain unmapped")
    generics = cases["generic"]["observations"]
    require(generics[0]["canonical_caller"] != generics[1]["canonical_caller"] and
            generics[0]["association"].get("site_id") == generics[1]["association"].get("site_id"),
            "generic instances must remain separate observations for the same physical site")
    site_ids = {s["site_id"] for s in sites["sites"]}
    joined = {o["association"].get("site_id") for o in observations}
    return {"cases": cases, "unicode_observation": witness,
            "unassociated_site_ids": sorted(site_ids - joined),
            "observation_count": len(observations),
            "association_counts": {state: sum(o["association"]["state"] == state for o in observations)
                                   for state in ["exact", "unique_containing", "ambiguous", "unmapped", "external"]},
            "limitations": ["Optimized monomorphized MIR may elide or inline physical calls even at opt0.",
                            "Macro expansion maps, receiver types and assertion reasons are unavailable.",
                            "An unassociated or absent observation is not evidence of safety.",
                            "No whole-program or arbitrary ambient Cargo coverage is claimed."]}


def capture(args, work, fixture, plan):
    for field in ["cga_driver", "cargo_cga", "cga_source", "expected_driver_sha256", "expected_launcher_sha256", "example"]:
        require(getattr(args, field), "capture requires --" + field.replace("_", "-"))
    driver, producer_identity, launcher = producer(args, work)
    binary = Path(args.example).resolve(strict=True)
    env = closed_environment(work)
    logs = work / "logs"
    logs.mkdir()
    sources = [{"file_path": r["path"], "text": (fixture / r["path"]).read_text()}
               for r in plan["inputs"] if r["kind"] == "source"]
    snapshot = example(binary, {"command": "snapshot", "root": str(fixture),
                               "source_paths": [s["file_path"] for s in sources]}, work, env, logs, "snapshot")
    write_json(work / "snapshot.json", snapshot)
    inputs = [{"path": r["path"], "text": (fixture / r["path"]).read_text()} for r in plan["inputs"]]
    query = [WRAPPER, "--workdir", fixture, "+" + TOOLCHAIN, "rustc", "--frozen", "--bin", "panic-pilot",
             "--target-dir", work / "identity-target", "--", "-vV"]
    verbose = run(query, fixture, env, logs, "rustc-identity").stdout.decode()
    require("commit-hash: " + RUSTC_COMMIT + "\n" in verbose, "unexpected rustc compiler identity")
    host = next(line.removeprefix("host: ") for line in verbose.splitlines() if line.startswith("host: "))
    summary = {"schema": "panic-pilot-acceptance-v1", "producer": producer_identity,
               "example_sha256": digest(binary.read_bytes()), "wrapper_sha256": digest(WRAPPER.read_bytes()),
               "rustc_verbose": verbose, "configurations": [], "mandatory_gate": "pending"}
    for config in plan["configurations"]:
        directory = work / config["name"]
        directory.mkdir()
        before = inventory(fixture)
        write_json(directory / "inventory-before.json", before)
        require(before == plan["inputs"], "fixture changed before capture")
        target = directory / "target"
        compile_env = dict(env, RUSTC=str(driver), CGA_INPUT='{"extract":true,"lint":null}',
                           RUSTFLAGS=" ".join(config["compiler_flags"]))
        cargo_args = ["build", "--frozen", "--no-default-features", "--bin", "panic-pilot",
                      "--target", host, "--target-dir", str(target), "-vv"]
        if config["features"]:
            cargo_args += ["--features", ",".join(config["features"])]
        command = [str(WRAPPER), "--workdir", str(fixture), "+" + TOOLCHAIN] + cargo_args
        try:
            result = run(command, fixture, compile_env, directory, "compiler")
        finally:
            after = inventory(fixture)
            write_json(directory / "inventory-after.json", after)
            require(before == after, "fixture input inventory changed during capture")
        require(digest(driver.read_bytes()) == producer_identity["cga_driver_sha256"] and
                digest(launcher.read_bytes()) == producer_identity["cargo_cga_sha256"], "producer binaries changed during capture")
        emitted = []
        fragments = directory / "exports"
        fragments.mkdir()
        for line in result.stderr.decode().splitlines():
            if line.startswith("cga-emit:"):
                path = Path(line.removeprefix("cga-emit:"))
                if not path.is_absolute():
                    path = fixture / path
                path = path.resolve(strict=True)
                require(path.is_relative_to(target), "compiler export escaped target directory")
                raw = path.read_bytes()
                retained = fragments / (str(len(emitted)) + "-" + path.name)
                retained.write_bytes(raw)
                graph = json.loads(raw)
                emitted.append({"path": str(retained), "sha256": digest(raw),
                                "has_main": any(n["name"] == "panic_pilot::main" for n in graph["nodes"])})
        write_json(directory / "exports.json", emitted)
        chosen = [e for e in emitted if e["has_main"]]
        require(len(chosen) == 1, "expected exactly one actual bin compiler JSON export")
        raw = Path(chosen[0]["path"]).read_bytes()
        (directory / "raw-export.json").write_bytes(raw)
        identity = {"schema_version": "panic-cga-capture-v2", "scope": "bounded_fixture", "provenance": "compiler_capture",
                    "graph_content_hash": snapshot["graph"]["graph_content_hash"], "capture_root": str(fixture),
                    "inputs": before, "rustc_verbose": verbose, "toolchain": TOOLCHAIN, "host": host, "target": host,
                    "exporter_revision": REVISION, "producer": producer_identity,
                    "packages": ["panic-pilot", "pilot-dep"],
                    "targets": ["bin:panic-pilot", "lib:panic_pilot", "lib:pilot_dep"],
                    "features": config["features"], "default_features": False, "all_features": False, "profile": "dev",
                    "cfg_flags": [v for f in config["features"] for v in ["--cfg", 'feature="' + f + '"']],
                    "compiler_flags": config["compiler_flags"], "cargo_args": cargo_args,
                    "environment": {key: compile_env.get(key) for key in ALLOWLIST},
                    "capture_command": command, "raw_export_sha256": digest(raw)}
        build_id = example(binary, {"command": "identity", "identity": identity}, work, env, directory, "identity")["build_id"]
        request = {"command": "import", "identity": identity, "expected_build_id": build_id,
                   "raw_export": raw.decode(), "graph": snapshot["graph"], "sources": sources,
                   "method_sites": snapshot["method_sites"], "inputs": inputs}
        evidence = example(binary, request, work, env, directory, "import")["compiler_evidence"]
        validate = dict(request, command="validate", compiler_evidence=evidence)
        require(example(binary, validate, work, env, directory, "validate")["valid"], "roundtrip invalid")
        controls = []
        for name in ["source", "config", "checksum", "reference", "ownership"]:
            bad = copy.deepcopy(request)
            if name == "source":
                bad["inputs"][0]["text"] += " "
            elif name == "config":
                bad["identity"]["compiler_flags"].append("-Copt-level=1")
            elif name == "checksum":
                bad["raw_export"] += " "
            elif name == "reference":
                graph = json.loads(bad["raw_export"])
                graph["roots"].append(len(graph["nodes"]) + 1)
                bad["raw_export"] = json.dumps(graph)
                bad["identity"]["raw_export_sha256"] = digest(bad["raw_export"].encode())
                bad["expected_build_id"] = example(binary, {"command": "identity", "identity": bad["identity"]}, work, env, directory, "negative-reference-identity")["build_id"]
            else:
                bad["method_sites"]["sites"][0]["caller_stable_symbol_id"] = "forged"
            example(binary, bad, work, env, directory, "negative-" + name, False)
            controls.append(name)
        executable = target / host / "debug/panic-pilot"
        runtime = []
        for mode in ["good", "option", "result", "index", "overflow", "debug", "feature", "macro", "dependency"]:
            expected_bad = mode in ["option", "result", "index", "macro", "dependency"] or (
                mode == "overflow" and config["overflow_checks"]) or (
                mode == "debug" and config["debug_assertions"]) or (
                mode == "feature" and bool(config["features"]))
            result = run([executable, mode], fixture, env, directory, "runtime-" + mode, expected_success=False)
            require(result.returncode == (101 if expected_bad else 0), "runtime control exit mismatch: " + config["name"] + "/" + mode)
            require(not expected_bad or b"panicked at" in result.stderr, "runtime bad control did not report a panic")
            runtime.append({"mode": mode, "expected_panics": expected_bad, "exit": result.returncode})
        check = acceptance(evidence, snapshot["method_sites"], fixture, config)
        write_json(directory / "acceptance.json", check)
        write_json(directory / "runtime.json", runtime)
        write_json(directory / "identity.json", identity)
        summary["configurations"].append({"name": config["name"], "build_id": build_id,
            "raw_export_sha256": digest(raw), "raw_export": str(directory / "raw-export.json"),
            "inputs_before_sha256": inventory_hash(before), "inputs_after_sha256": inventory_hash(after),
            "runtime_controls_passed": len(runtime), "negative_controls_passed": controls,
            "acceptance": str(directory / "acceptance.json"), "observation_count": check["observation_count"],
            "association_counts": check["association_counts"]})
        write_json(work / "summary.json", summary)
        print("captured " + config["name"], file=sys.stderr, flush=True)
    require(len({c["build_id"] for c in summary["configurations"]}) == 5, "configuration identities collided")
    # A valid independently captured configuration cannot be substituted for a
    # different expected build ID, even when the graph/source snapshot matches.
    request["expected_build_id"] = summary["configurations"][0]["build_id"]
    example(binary, request, work, env, logs, "negative-cross-config", False)
    require(inventory(fixture) == plan["inputs"], "final fixture inventory changed")
    require(digest(binary.read_bytes()) == summary["example_sha256"] and
            digest(WRAPPER.read_bytes()) == summary["wrapper_sha256"], "capture runner changed during acceptance")
    summary["mandatory_gate"] = "passed"
    write_json(work / "summary.json", summary)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", type=Path, required=True, help="new output directory outside the workspace")
    parser.add_argument("--prepare-only", action="store_true", help="materialize exact source corpus and matrix without compiler capture")
    parser.add_argument("--cga-driver")
    parser.add_argument("--cargo-cga")
    parser.add_argument("--cga-source", help="checkout of the exact pinned producer revision")
    parser.add_argument("--producer-patch", help="exact git diff --binary HEAD bytes for a patched producer")
    parser.add_argument("--expected-driver-sha256")
    parser.add_argument("--expected-launcher-sha256")
    parser.add_argument("--example", help="prebuilt panic_evidence example executable")
    args = parser.parse_args()
    work = args.work_dir.absolute()
    # Resolve the existing parent first to reject symlink aliases into a repo.
    work = work.parent.resolve(strict=True) / work.name
    owned = False
    try:
        fixture, plan = prepare(work)
        owned = True
        result = plan if args.prepare_only else capture(args, work, fixture, plan)
        print(json.dumps(result, ensure_ascii=False))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        # Never turn a compiler/capability failure into a synthetic GREEN or a
        # skipped mandatory gate. Retain failed logs and partial results.
        if owned and not args.prepare_only:
            write_json(work / "blocked.json", {"mandatory_gate": "blocked", "reason": str(error)})
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
