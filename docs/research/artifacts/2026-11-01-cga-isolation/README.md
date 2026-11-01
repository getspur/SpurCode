# CGA generic drop-glue isolation — 2026-11-01

**A four-line library reproduces the exact CLI compiler ICE with the original pinned CGA driver and no dependencies.** CGA replaces a concrete generic trait argument with an identity parameter while expanding a virtual call. Traversal subsequently asks rustc for `std::ptr::drop_glue::<S>` while `S` is still unresolved. This dossier is investigation evidence for Beads `bd-3qw`; it contains no production fix. The issue remains `in_progress` for root review.

```rust
pub trait Trait<S> { fn consume(&self, value: S); }
impl<S> Trait<S> for () { fn consume(&self, _value: S) {} }
#[no_mangle]
pub fn entry(object: &dyn Trait<()>) { object.consume(()); }
```

The explicit exported `entry` makes this library observable under the pinned producer's `NoConstOrStatic` root policy. The ordinary Rust program supplies `()`, which needs no destructor. The analysis introduces `S`; the input does not request unresolved generic code generation. This is a small verified reduction, not a proof of textual minimality.

## Controlled observations

All seven cases actually invoked the compiler, sequentially, in fresh scratch packages and target directories. The harness checks exits, the exact ICE signature, fresh export markers, graph roots and call coverage, edge references, and the retained generic parameter in the reference control. These are compiler experiments, not runtime tests or a TDD fix validation.

| Original source / control | Compiler and CGA input | Exit | Fresh graph |
|---|---|---:|---|
| Four-line reproducer | Original driver; `extract:true` | 101 | None; exact `drop_glue::<S>` ICE |
| Same bytes | Original driver; `extract:false` | 101 | None; same ICE; analysis still enabled |
| Same bytes | Original driver; `CGA_INPUT` unset | 0 | None; analysis-disabled control |
| Same bytes | Pinned ordinary rustc; CGA input present but unused | 0 | None; normal compiler control |
| Static dispatch, otherwise same generic trait/impl | Original driver; `extract:true` | 0 | 1 root, 2 nodes, 1 `Called` edge |
| Nongeneric trait with concrete unit argument | Original driver; `extract:true` | 0 | 1 root, 3 nodes, `Called` + `VTableCalled` |
| Generic argument passed by reference | Original driver; `extract:true` | 0 | 1 root, 3 nodes, `Called` + `VTableCalled`; **still contains `Trait<S>`** |

The reference control avoids the failing owned-value drop, but exposes the unresolved parameter in an exported node. Its successful compilation is not evidence of correct monomorphization. Disabling analysis is likewise not a successful graph capture.

## Exact producer and failure path

The producer is CGA `2fc63108f0b674fe7a178191ffa15647a18879f1`, with only the retained [macOS rpath build patch](upstream/original-macos-rpath.patch), SHA256 `72f3d4cd64439d988deba0ee1e0f449df8290e6f1d1e0ae83ba82d97b1592aca`. Original `cga-driver` SHA256 is `5647b7f2c7f857dda0dd0204e91d870287988181ca5ec4d7792810a561886172`; original `cargo-cga` SHA256 is `7f84bc715138c09744fd351859ca02b240ea768d9756069229e9250fa9447b7a`. Neither binary nor its checkout was changed. No diagnostic producer was built or used; the original driver's existing logger supplied the instance trace. The launcher was never invoked.

The toolchain is `nightly-2026-10-18`, rustc commit `8006091eb71ae7d9e9bccb66c6122c9d68af4466`, target `aarch64-apple-darwin`. Every compiler experiment retains these explicit flags:

```text
-Zalways-encode-mir -Zhint-mostly-unused -Copt-level=0 -Cpanic=unwind
-Cdebug-assertions=on -Coverflow-checks=on -Cforce-frame-pointers=yes
```

The reproducer is a standalone Rust 2021 library with its own workspace and lockfile. Its fresh scratch working directory omits the repository Cargo config ancestry, dependency tree and lint policy. Cargo supplies `-C debuginfo=2`, while the CLI supplies `-C debuginfo=line-tables-only`; this difference is explicit in retained compiler invocations. No required flag or check was weakened. The seven fixture comparisons share the same manifest, flags and environment except the stated compiler/input/source change and necessary output paths. `RUST_BACKTRACE=full` and `RUSTC_LOG=cga_analysis::post_mono=info` are observation controls.

The standalone trace records:

```text
entry
  Called virtual Trait::consume, args=[dyn Trait<()>, ()]
  VTableCalled impl::consume, args=[S/#1]
  visit_instance impl::consume, args=[S/#1]
  PostMonoNode::visit_terminator
  Instance::resolve_drop_glue -> Instance::expect_resolve
  ICE instance.rs:640:21: failed to resolve instance for std::ptr::drop_glue::<S>
```

The exact Cargo CLI build also reproduced the ICE (exit 101, 6.92 seconds) with a full backtrace. A second trace replays **only its exact Cargo-emitted `spur_cli` compiler invocation**, directly through the original driver, to avoid rebuilding dependencies (exit 101, 8.12 seconds). This is explicitly a compiler replay, not a second Cargo build. Its trace identifies the corresponding real virtual-call expansion:

```text
tower_layer::Layer::layer, with concrete service argument
  VTableCalled octocrab::service::middleware::auth_header::{impl#1}::layer
  args=[S/#1]
  failed to resolve instance for std::ptr::drop_glue::<S>
```

The real implementor is `impl<S> Layer<S> for AuthHeaderLayer` in Octocrab 0.50.0; its `Self` type has no generic parameter. Tower 0.5.3's `BoxCloneSyncServiceLayer::layer` calls `self.boxed.layer(inner)` at line 113. The generic owned `inner: S` and its cleanup are sufficient for the failure. The retained trace establishes the erroneous implementation instance and failing drop type; it does not record an exact MIR place/basic-block number for that CLI drop.

## Source-backed explanation

[source-contract.json](source-contract.json) pins the indexed upstream selectors and exact Git snapshots:

1. [`post_mono_pass`](upstream/cga/bin/cga-driver.rs) lines 28–70 first invokes rustc's normal collector. When `CGA_INPUT` exists, it always collects roots and runs `generate_callgraph`; `extract` controls only serialization.
2. [`dyn_object_call_to_impl_methods`](upstream/cga/cga_analysis/src/dyn_object.rs) lines 6–58 accepts concrete call arguments as unused `_args`. It enumerates trait implementors, filters generic **self types**, then builds trait arguments from the chosen self type plus `GenericArgs::identity_for_item(tcx, virtual_def_id).iter().skip(1)`. A nongeneric self type with a generic **trait argument** passes this filter. Identity `S/#1` replaces the actual `()` in the reproducer, or the concrete service type in the CLI.
3. [`PostMonoNode::visit_virtual` and `on_edge`](upstream/cga/cga_analysis/src/post_mono.rs) pass those returned instances into recursive traversal. The logs directly observe this transition; it is not inferred from symbol names alone.
4. The same file's `get_call_def_mir` drop arm calls `monomorphize_args(place.ty(...))` and then `Instance::resolve_drop_glue(tcx, ty.ty)`. `monomorphize_args` instantiates using `self.instance`, whose argument is already unresolved. The exact ICE and backtrace show the resulting unresolved drop request. The earlier `RootCollector::visit_drop_use` candidate is not the failing stack frame.

External package MCP reads grounded the CGA selections. Worktree `knowledge_context_pack_2` failed analyst candidate preparation. Octocrab/Tower external lookups reported their revisions unavailable, so bounded cached-source excerpts were checked byte-for-byte against cached `.crate` members and their Cargo.lock checksums. No fetching, indexing, upgrading or graph publication occurred.

This isolates the argument reconstruction fault on this path. It does not establish a complete correction for all implementor types, associated types, bounds or vtable coverage. A later producer change should preserve and resolve concrete trait/implementor substitutions. A useful regression would require this owned-value input to compile **with analysis enabled**, export a concrete `Trait<()>` implementation, and reject residual generic parameters; the reference control should also export a concrete node. No such fix is claimed or tested here.

## Replay

Validate retained evidence without a compiler, network or mutable build cache:

```bash
python3 docs/research/artifacts/2026-11-01-cga-isolation/replay.py
```

Rerun all seven compiler controls on this pinned macOS environment:

```bash
python3 docs/research/artifacts/2026-11-01-cga-isolation/run.py \
  --output /private/tmp/cga-isolation-fresh
```

For one direct offline SPUR-wrapper reproduction, run from the repository root (a fresh scratch target avoids cached evidence):

```bash
env PATH=/Users/kevintruong/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin \
  RUSTUP_HOME=/Users/kevintruong/.rustup CARGO_HOME=/Users/kevintruong/.cargo \
  RUSTUP_TOOLCHAIN=nightly-2026-10-18 SPUR_REMOTE=0 SPUR_SCCACHE_S3=0 \
  RUSTC=/private/tmp/spur-cga-pilot-20261031/target/release/cga-driver \
  RUSTC_WRAPPER= CARGO_INCREMENTAL=0 LC_ALL=C RUST_BACKTRACE=full \
  CGA_INPUT='{"extract":true,"lint":null}' \
  CARGO_TARGET_DIR=/private/tmp/cga-isolation-single-target \
  RUSTFLAGS='-Zalways-encode-mir -Zhint-mostly-unused -Copt-level=0 -Cpanic=unwind -Cdebug-assertions=on -Coverflow-checks=on -Cforce-frame-pointers=yes' \
  scripts/spur-cargo build --frozen --offline \
    --manifest-path docs/research/artifacts/2026-11-01-cga-isolation/repro/Cargo.toml \
    --lib --no-default-features --target aarch64-apple-darwin -vv
```

Expected exit: **101**, with the exact generic drop-glue ICE. The full harness pins its driver hash and rustc commit, copies inputs into scratch packages, verifies all seven observations, and records every command/environment and actual process exit. All Cargo commands use `SPUR_REMOTE=0 scripts/spur-cargo`; the ordinary rustc control uses that wrapper too.

## Evidence and limits

[report.json](report.json) is the compact machine-readable result. [capture-index.json](capture-index.json) inventories exact archive members; `captures.tar.gz` preserves complete new logs, command receipts, input inventories and raw passing graphs. [cli-failure-tail.txt](cli-failure-tail.txt) is a bounded byte-range excerpt of the full CLI trace; its offsets and parent hash are recorded. [evidence-index.json](evidence-index.json) hashes the retained artifacts.

One attempt to enable verbose logging through the full Cargo build invalidated dependency fingerprints. Only that worker-owned Cargo process was interrupted; its actual exit was `-2` (SIGINT), after 108.86 seconds. It is retained as an interrupted observation, not a compiler result. The subsequent direct compiler replay completed and produced the needed trace. The initial probe and both completed seven-control runs are also preserved, without overwriting earlier captures.

Historical CLI evidence was compiled at `5c75446f1f01b0a5b70a48426f9b51dd822a2349`; these new CLI replays used main HEAD `a9ee997c925d243cd1bbe4e4895867c2ff11b70e` with its five protected user edits. Fresh before/after inventories contain 3,537 tracked non-notebook main files, unchanged across this investigation. They do not claim bit-identical historical inputs or a fully closed native/dependency build environment. Notebook files were only hashed. The original producer identities, completed prior dossier, five protected main-file hashes, CURRENT target `64c5b7a163b7c832c537236915db12b7cb7c6591e8e45bd072ea0404c826d8c4.parquet`, and manifest SHA256 `1b9275fc691c61f990e278c8d3bcb2748dd03068a2dabd17b31750e2b8f50f56` remain unchanged.

There is no successful full CLI capture, `spur::main` export, runtime panic demonstration or whole-workspace safety conclusion. Root owns Notebook integration, final review and issue closure.

The worker's completion audit RPC rejected assignment validation again, while a fresh issue lookup still showed this dispatched worker assigned and `in_progress`. [worker-audit-request.json](worker-audit-request.json) preserves the exact complete request; [worker-audit-error.txt](worker-audit-error.txt) preserves the exact rejection. Root was notified through `report_progress` to persist and reload that request through its authorized issue API. The worker did not write the Beads database or change issue state. Audit persistence remains a root handoff item.
