# M5 current packaging entry gate

Canonical task **bd-2zan**, `shipped-packaging`, plan
`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`. Routing stays
**codex / profile codex / gpt-6.1-sol / xhigh**. This source-bound PRE is supplied
before M4 approval and M5 edits. The user has already approved implementation
and local validation; no repeated permission question is needed.

Read [the root M4 review](../root-M4-review/README.md), then run:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M5-pre/verify_pre.py
```

Reload **sol_f5632ddd6f8147a5**, **sol_7381ebd0b1984d1b** (SAT) and
**sol_adb5223797974d16** (expected UNSAT). Complete requests, responses, generic
lowerings and reloads are in `proof.json.gz`. The inspected Jev gate is open
(weakest .78); interpretation confidence is not proof. Independent validation
checks all nine compound predicates over14 declared requirements, including
the host-only package violation. These are PRE requirements, not packaging
runtime acceptance. No new numeric policy defaults are authorized.

The **62-file** entry fingerprint is
`a8babca9fa52809ff2a76061b43852ab6b9fe866b0497135f2ee07a59207d880`,
including current M4 source plus actual wrapper/npm/xtask/release/CLI-test paths.
`source-inventory.json` records that the wrapper and npm installer do not yet
mention the companion and cargo-dist lists only spur-cli. Actual ordinary helper
metadata is verified at this entry: package1.24.0/protocol2/macOS/aarch64, clean
exit0, no native cache creation. Its SHA is
`9463cc1a394ac4dc2fd49911933344344cb22f5d4c564215abc7f2d95f635cfa`.
Older source091 PREs are historical and must not be run against M5 edits.

Implement the required host/helper pair throughout normal source builds,
local installs, xtask distribution, both release paths and npm installation.
Keep compatible siblings in the same bin directory and verify expected checksums,
package version, protocol, OS/architecture and actual runtime payload. On macOS
the approved helper embeds the verified dylib at build time and materializes it
under its child-scoped cache; do not assume it requires a loose dylib beside the
host or change its pinned runtime/profile without a supplemental grounded gate.
Fix each packaging seam that needs it; demonstrate the other paths rather than
making unrelated edits just to touch every listed file.

Preserve native host behavior: no runtime Cargo/compiler/executable fetch and
no native inference in its normal/build closure or actual loaded runtime. The
existing npm installer/lazy installation route may fetch verified release assets
before host launch; it must install or repair a consistent pair or fail clearly.
Test helper absence, mismatch and failed checksum without silently accepting a
host-only upgrade. Use local controlled fetch/release fixtures, never actual
publication or global installation.

Current main's independent xtask/wrapper/release changes are present in this
entry and hash-verified. Preserve them. The old unreviewed normalized commit
`3406dc24a975f3bcc528da4c3ee34dbb682e07c7` contains extensive unrelated history;
**never cherry-pick it wholesale**. Inspect only its c1596d8c0-relative xtask
delta,119 additions/8 deletions, preserved exactly in
`historical-xtask-only.diff.gz`. Reuse compatible pieces after appropriate RED,
reconciled with current entry bytes. Also inspect/reuse the npm layout regression
from test commit `726ea20780f51c8935f2bb3ead608e0ea9f07188` after this PRE.

Commit tests and observe runtime assertion RED for wrapper/install/npm/release
layout before the minimal fix. Include space/Unicode paths, missing/incompatible
helper, checksums, default features and no runtime compilation. Preserve existing
skill/package assets and unrelated TUI/xtask changes. All production changes
still need GREEN, fresh root Jev/catalog POST and final-node PRE before approval.

Run the **unchanged DEFAULT embedding-enabled**
`analyst_temporal_views::analyst_build_emits_external_dependency_surface_views`
test. Its existing query and all five rows, including repeated serde::Serialize,
are the acceptance condition. No no-default-features alternative, weakened query,
changed assertion or fault-worker substitute can satisfy it. The test file's
entry hash is included in the source gate. Exercise the actual normal CLI and
helper from an installed **private temporary package layout**, including real
product embeddings matching pinned references within1e-4 and normal shutdown.
Record exact feature sets, artifact/loaded-library identities and cleanup
statuses. Test-only native feature unification must not be called normal-host
evidence. Preserve typed missing/mismatch/crash failures and existing owners.

`scripts/cloud-build` is an external symlink into ../spur-notebook. Do not edit
through it or modify that project. Tests may inspect exact necessary external
inputs and record hashes. If a concrete external change is unavoidable, report
the seam first and continue independent authorized in-repository work. The bundle
publisher contains a real S3 upload command: test with intercepted commands or
another local-only route; do **not** upload, publish npm/GitHub assets or restart
services. No installed binary overwrite, old/live notebook edits or graph/CURRENT
change is authorized. Model source caches remain read-only; use private copies.

Use scripts/spur-cargo and the approved local environment, one Cargo writer.
Validate target matrix behavior with focused fixtures, but report only actual
macOS ARM64 execution here; static tests do not certify other platforms. Keep
worker MCP progress heartbeats and recoverable commits. Verify the fresh canonical
dispatch and persist/replay an authorized INTENT before edits/builds. ROOT has
no Cargo writer running at release. Leave the issue in progress; root supplies
the final runtime/performance/containment/fresh Rust Notebook PRE before approval.
