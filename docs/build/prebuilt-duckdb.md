# Prebuilt DuckDB in remote builds

DuckDB remains pinned to **1.4.4** for the analyst extension ABI. Default builds
still enable `duckdb-bundled` and compile DuckDB from source. The analyst, core,
and CLI expose that feature explicitly so application builds can instead link
verified prebuilt libraries (static on Linux/macOS, a DLL on Windows MSVC).

The shared `spur-notebook/scripts/cloud-build` pipeline supports this mode:

```sh
SPUR_DUCKDB_PREBUILT=1 SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 \
  scripts/spur-cargo build --locked -p spur-cli --no-default-features \
  --features embed,tui-default,interactive-default
```

For a smaller upstream evaluation, build `spur-analyst --no-default-features`.
The helper verifies Cargo's selected feature graph before downloading anything:
`libduckdb-sys` must be exactly 1.4.4 and its `bundled` feature must be absent.
Enabling `SPUR_DUCKDB_PREBUILT` cannot cancel an enabled Cargo feature. Every
selected package, including its test dependencies, must agree on linkage.

The shared build pipeline must include `_ensure-duckdb.py`; it is transferred
explicitly to the VM because `scripts/cloud-build` can be a sibling-repository
symlink. The mode is currently supported through remote `scripts/spur-cargo`
commands. SpurLab also supports Windows x64 pnpm/Tauri installer packaging.

## Artifacts and platforms

The provisioner downloads official `duckdb/duckdb` v1.4.4 release assets and
verifies pinned SHA256 digests. It merges the engine and support `.a` archives,
then supplies `DUCKDB_LIB_DIR`, `DUCKDB_INCLUDE_DIR`, `DUCKDB_STATIC=1`, and a
`duckdb.pc` linking the system C++ runtime. It does not compile DuckDB C++ source.
Rust bindings and application code still compile normally.

| Rust target | Release archive | C++ runtime |
| --- | --- | --- |
| aarch64-unknown-linux-gnu | static-libs-linux-arm64.zip | system libstdc++ |
| x86_64-unknown-linux-gnu | static-libs-linux-amd64.zip | system libstdc++ |
| aarch64-apple-darwin | static-libs-osx-arm64.zip | system libc++ |
| x86_64-apple-darwin | static-libs-osx-amd64.zip | system libc++ |

Windows MSVC uses `libduckdb-windows-amd64.zip` or
`libduckdb-windows-arm64.zip`, with pinned checksums and `DUCKDB_STATIC=0`.
The helper stages `duckdb.dll` beside application and test executables; ship
the DLL with the executable. The `.lib` file is an import library, not a static
engine. Musl and Windows GNU remain unsupported. macOS cross builds continue
to use `scripts/spur-cargo zigbuild` and its system-libc++ linker setup.

For Windows x64, use `SPUR_BUILD_ARCH=x86_64` and
`scripts/spur-cargo xwin test --locked --release -p spur-analyst
--target x86_64-pc-windows-msvc --no-default-features
--test duckdb_linkage --no-run` with the same prebuilt/remote environment above.
Run the resulting test executable with its adjacent DLL on native Windows,
passing `--ignored --nocapture`. Cross compilation alone is not runtime evidence.

Pass a single explicit `--target` for cross builds. If a target is selected in
Cargo configuration, repeat it on the command line; the provisioner deliberately
rejects that implicit configuration instead of guessing Cargo's merged target.

The cache is under `$XDG_CACHE_HOME/spur-duckdb` (the remote pipeline sets
`XDG_CACHE_HOME=/mnt/cargo/.cache`), keyed by version, target, digest, and recipe.
A target lock and atomic directory publication keep concurrent builds from
observing partially prepared libraries. DuckDB extensions retain the existing
`v1.4.4/<platform>/` directory layout; they are not part of the engine archive.

## Verification

```sh
bash scripts/test-duckdb-linkage.sh
SPUR_DUCKDB_PREBUILT=1 SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 \
  scripts/spur-cargo test --locked -p spur-analyst --no-default-features \
  --test duckdb_linkage -- --ignored --nocapture
```

The integration test requires signed DuckPGQ/Onager extensions in the existing
cache, the configured vendored directory, or accessible extension repositories.
It asserts both extensions load, a property-graph edge query and Onager PageRank
succeed, and a DuckPGQ error returns without aborting the process. A second
test covers JSON extraction, transaction rollback, and a Parquet round trip.
Ordinary SQL success alone is insufficient: production query paths can fall back to recursive SQL when
DuckPGQ is unavailable.
