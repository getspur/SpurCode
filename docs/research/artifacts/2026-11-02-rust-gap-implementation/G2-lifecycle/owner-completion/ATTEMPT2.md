# O1 attempt 2 — work in progress, not acceptance

Assignment: `bd-3lcp`, plan task `ort-owner-constructors`, delegation
`8b8bf48356d746ea`. Binding audit 2503 preceded edits. The inherited O1
README, raw logs, and authored refs remain unchanged. The current branch has
the following test-first work, with detailed raw output in the adjacent logs:

| Work | RED test commit | Fix commit | Evidence |
| --- | --- | --- | --- |
| Compiler environment lease | `bbb3bb49e` | `b2d8689c6` | `o1-compiler-{red,green}.log` |
| Allocator memory-info backing | `794e25630` | `e28c7c7f3` | `o1-info-{red,green}.log` |
| Device adapter exclusion | `39a7d0052` | `c3632aaea` | `o1-adapter-{red,green}.log` |
| Editor initializer exclusion | `8618c3364` | `c644e97b7` | `o1-editor-{red,green}.log` |
| Kernel attribute Clone exclusion | `9fb196e5e` | `5d864f2a8` | `o1-clone-{red,managed-green,nonmanaged-green}.log` |
| Partial tensor cleanup | `7aea80fe6` | `eecac0e1d` | `o1-tensor-{red,green,green-v2}.log` |
| Sequence extraction allocator | `7fea3db4f` | `56f37027c` | `o1-sequence-{red,green}.log`; audit 2512 and correction 2514 |
| Editor node attributes exclusion | `8e4137bcb` | `b4d4084c1` | `o1-node-{red,green}.log` |
| Published compiler output on error | `11d4b0cef` | `a16d84bb4` | `o1-compiler-buffer-{red,green}.log`; audit 2517 |

The tensor tests create actual native values, inject failure immediately before
the relevant fallible operation, and count calls made to native `ReleaseValue`.
This does not simulate a native status table and does not establish final
release ordering. The sequence RED inspects the allocator owner's strong count
while the allocator remains live, then drops the extracted values before the
allocator; it does not dereference a dangling pointer. The node RED intercepts
the native call, since the pinned native implementation consumes and overwrites
the supplied attribute pointers. Managed attribute-free nodes remain usable.

## Verification at source hashes in `attempt2-source-hashes.json`

All graph commands used `SPUR_REMOTE=0`, `TMPDIR=/private/tmp`, target
`/private/tmp/rust-gap-g1-acceptance-target`, and unset the ORT, DYLD,
`RUSTFLAGS`, update and bless overrides shown in the commands in this task.
The isolated clone fixture used `/private/tmp/rust-gap-n0-attempt3-wrapper-target`;
its harness manifest and lockfile are copied here.

* `scripts/spur-cargo test -p spur-graph --test ort_owner_constructors --test ort_custom_allocator --test ort_shutdown -- --nocapture`: exit 0, `o1-final-combined-v5.log`. The constructor binary reports 21 entries: 12 parent cases and nine directly run no-op child entries. The custom-op binary reports one parent and one child entry. The shutdown binary reports seven parent cases and seven child entries; nested child output is not additional independent cases.
* `scripts/spur-cargo check -p spur-graph --all-targets` and the same command with `--no-default-features`: exit 0, `o1-final-{default,nodefault}-v5.log`.
* The copied clone harness compiled with non-managed, managed, and training-only features: exit 0. Managed plus training exited 101 with the intended diagnostic. See `o1-final-{nonmanaged,managed,training,forbidden}-v5.log`.
* Workspace `scripts/spur-cargo fmt --all --check` and changed Rust `git diff --check` exited 0. Baseline and patched wrapper files were formatted in temporary copies using the same `rustfmt --edition 2021 --config skip_children=true`; `git diff --no-index --check` exited 1 because files differ and emitted no whitespace diagnostics. Historical whole-base whitespace is separately recorded in the inherited `preexisting-whitespace.log`.
* `o1-provider.log` records actual unavailable and registration-error results for the five accelerator providers in the pinned macOS CPU runtime. Those results do not classify other native builds.

## Inventory and limits

`managed-resource-ownership-attempt2.json` is a working 48-row inventory;
two rows split out the CPU adapter and newly excluded editor node attributes.
`Session::overridable_initializers` leaks its native allocated name and remains
unresolved (audit 2520). The tentative `ModelMetadata::custom_keys` concern in
that audit was refuted by the wrapper and native source (audit 2521).
`native-source/attempt2-manifest.json` identifies pinned native
source files and hashes. `attempt2-discover-owners.py` maps the explicit
creation, selected native getters, and selected wrapping syntax it discovers;
it does **not** prove coverage of macro expansions, aliases, or every native
entry point. The original inventory and old POST receipts remain historical.

The independently authored PRE requests and reloaded receipts are in Beads
audits 2504, 2505, and subsequent per-group audits. The persisted/reloaded
`attempt2-v5-post-*.json` files check thirteen source-derived predicates and
reject missing-row, missing compiler lease, wrong backing, missing cleanup,
missing exclusion, and unsupported-as-supported mutations. A separate
`attempt2-inventory-post-*.json` pass checked the earlier 46-row snapshot and
its controls against a separately authored policy. That policy was reconstructed
after implementation, and the snapshot predates the two newly discovered rows;
it is therefore not the required pre-frozen, complete inventory-wide POST.
None of these finite checks proves Rust/native memory safety. Full inventory
coverage reconciliation, independently frozen complete expected policy and a
complete source/test-bound POST remain outstanding. The editor session lease
handoff also lacks a dedicated failure-first finalization regression. O2 owns
the separate final release-order and exactly-once acceptance. Do not approve,
close, merge, or dispatch O2 on this attempt as it stands.
