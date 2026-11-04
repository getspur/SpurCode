# O1 allocator guard — incomplete owner acceptance

Assignment: bd-3lcp, plan task `ort-owner-constructors`, delegation
`60d9b6b1e3c84302`, base `c7231995af2d601041b20142545cde4547780636`.
Binding audit 2467 preceded edits. The inherited attempt-three README, logs,
upstream ledger and protected refs remain unchanged. This is not N0, O1, or G2
acceptance: the inventory has unresolved paths listed below.

## Change and test

`KernelAttributes::allocator` and `KernelContext::allocator` return a
descriptive error under `managed-environment`. Their native calls and pointer
handling are inside `cfg(not(feature = "managed-environment"))`, so those
calls are not compiled into the managed branch. The non-managed native bodies
are unchanged. The stale session-only comment on `Allocator::from_raw` is
corrected. This does not exclude the rest of the custom-operator API.

Test commit `982bd4e1b` preceded fix commit `ec55a83ae`. The small ONNX fixture
is derived from `mul_1.onnx`: its node is a real `test.custom:Probe` custom op.
ONNX Runtime supplies valid kernel-info and kernel-context objects to the
callbacks. The probe records both outcomes without panicking across FFI, runs
synchronous inference and checks its output before shutdown. RED exit 101 in
`custom-allocator-red.log` reports both calls returned success; GREEN exit 0
in `custom.log` has one driver and one child-entry test. The child entry when
run directly without its cache variable is a no-op. Zero allocator acquisition
is established by the compiled cfg branch and observed error, not by a native
call counter; no native acquisition instrumentation was added.

## Verification

All local builds used `SPUR_REMOTE=0 TMPDIR=/private/tmp` and, for graph,
`CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target`. No ORT, DYLD,
RUSTFLAGS, update or bless overrides were present in the inherited environment;
the commands explicitly unset those variables as well. The graph checks ran
serially. The wrapper matrix used an isolated `/private/tmp/ort-o1-matrix`
copy with an added empty workspace declaration and the separate
`/private/tmp/rust-gap-n0-attempt3-wrapper-target` target.

* `scripts/spur-cargo test -p spur-graph --test ort_custom_allocator -- --nocapture`: exit 0, one substantive parent (and one child entry), `custom.log`.
* `scripts/spur-cargo test -p spur-graph --test ort_shutdown -- --nocapture`: exit 0, seven substantive parent cases plus seven child entries, `shutdown.log`; includes the existing managed async rejection and synchronous inference/PGQ cases.
* `scripts/spur-cargo check -p spur-graph --all-targets`: exit 0, `default.log`.
* `scripts/spur-cargo check -p spur-graph --no-default-features --all-targets`: exit 0, `nodefault.log`.
* `scripts/spur-cargo check --manifest-path /private/tmp/ort-o1-matrix/Cargo.toml --no-default-features --features std,load-dynamic --offline`: exit 0; adding `training` also exits 0. Adding both `training,managed-environment` exits 101 with the intended compile error. See `matrix-*.log`.
* `scripts/spur-cargo fmt --all --check`: exit 0, `workspace-fmt.log`. The isolated full wrapper `fmt --check` exits 1 because the vendored upstream formatting is not rustfmt-default; raw `wrapper-fmt.log` preserves the failure. The baseline copy also exits 1, `wrapper-fmt-baseline.log`. Both baseline and patched isolated copies were formatted successfully; `normalized-kernel.diff` shows the normalized delta. The original vendored source was not wholesale reformatted.
* `git diff --check c7231995af2d601041b20142545cde4547780636 HEAD` and the scoped changed-source check: exit 0. This compares only changes since the dispatched base. The inherited diff `git diff --check 114280493 c7231995af2d601041b20142545cde4547780636` exits 2 on historical raw logs and upstream README whitespace; see `preexisting-whitespace.log`.

The new raw logs, upstream native-source copies and captured normalized diff
are preserved byte-for-byte and contain whitespace that makes an unrestricted
evidence-only `git diff --check` exit 2. They are distinct from the clean
changed Rust source check.

Graph checks emitted existing warnings. Host: Darwin arm64, rustc 1.94.1.
The custom regression binary SHA-256 is
`0a8a981970415d2598dc68edb7952bdebb988df04b5161b4f37186d64cdb0397`.
The shutdown binary SHA-256 is
`39223baeade6b193a543d02110d164a136da2da9100dd38b804c832629d5d6fa`.
Final kernel SHA-256 `7a1ec399fedda672a361938972e42fac03a5625e426c55015f218ae561265e19`;
memory `bd3d1db1d09d8f1b1a9a07cb0166a192653a82646d6fcf107661ff9cba60f209`;
test `e1043e975faab932873d47deedc412703640adc860b508b1da0017db8046d890`;
fixture `9a4a9efe5a08f5ed2e83a2e23f3ceb5972315bad34ed61042aaf7fa1c44309e2`.

## Inventory and finite checks

`managed-resource-ownership.json` is deliberately partial, with file hashes,
source symbols, release/backing descriptions and explicit `unresolved` rows.
`native-creation-search.txt` records a broad raw search, including non-owning
false positives; it is a lead list, not proof of exhaustiveness. In particular,
`Graph::add_initializer` transfers an owned native value while the wrapper's
environment guard drops; `Adapter::from_file`/`from_memory` pass a device
allocator without retaining it; `ModelCompiler::new` drops its temporary Env
guard; `KernelAttributes::clone` owns a copied native info without a guard;
and `Allocator::memory_info` exposes a borrowed native pointer without an
allocator lifetime. Both `DynTensor::new_inner` and
`Tensor<String>::from_string_array` can return an error after native value
creation but before a Rust owner is installed, leaving the value untracked.
Signals and audits 2472/2477/2481/2485 record the findings. The pinned
ORT 1.22 header confirms graph ownership of the initializer. Native source at
release commit `f217402897f40ebba457e2421bc0a4702771968e` confirms that
`ModelCompilationOptions` stores an Env reference, adapter allocator wrappers
retain a raw `OrtAllocator*`, allocator `Info` returns a pointer into the
underlying allocator, and `OpKernelInfo` is a non-owning view. The exact source
files and hashes are preserved in `native-source/` and audit 2490. These
paths have no matching Rust prerequisite owner; no runtime crash was induced.
The graph initializer's native environment dependence remains unproven.
Header SHA-256
`d683537d0fdc29e977b5520f7f15d87a0ac212ae6d94fa9be8893a52655621ae`.

`pre.json` contains the catalog request and reloaded receipt
`sol_400bc7ea14ad43a9` (sat/pass), audited as 2471 before edits. The three
`post-v2-*.json` files contain complete catalog requests, results and reloaded
receipts over the **18 classified rows of the current 44-row ledger**: actual
`sol_a5a886e153cf44f8` sat/pass, wrong allocator backing
`sol_72efcec9e6aa427f` unsat/fail, and missing custom allocator exclusion
`sol_b0bed745f6d74136` unsat/fail. Audit 2486 contains those full records.
The earlier `post-*.json` and audit 2478 are preserved as prior evidence,
but predate the expanded ledger and do not certify the current snapshot.
The expected pairs come from the reviewed snapshot itself, so these finite
checks catch the demonstrated mutations but do not independently prove the
source classification, native lifetimes, other constructors, or all schedules.
`verify-inventory.py` checks all 44 current file hashes, the three outcomes,
that the classified request rows agree with the current ledger, and the two
managed cfg branches reject before the native calls. Later native-source
findings changed only unresolved-row annotations; they did not change the
classified POST inputs.
The requested complete inventory and full source-bound POST remain outstanding.
