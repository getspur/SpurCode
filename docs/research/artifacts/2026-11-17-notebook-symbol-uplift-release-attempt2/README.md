# U5 attempt 2: reader validation documentation correction

Task **bd-c1v3**, plan U5. Documentation-only rework on
`040dc7ba665e6f0a449228cbdea205d02949c643`; the five original commits
`fe06191df`, `43a96744d`, `779b5a1e6`, `5545f6d1e`, `040dc7ba6`
remain ancestors of this worker branch. No merge or issue closure.

Rationale: describe the reader's actual validation boundary while keeping
native publication, consumer behavior and all public interfaces unchanged.

[wording.patch](wording.patch) is the complete public documentation correction:

> `read_result` checks the declared consumer/native contract metadata and
> source identity scheme, exact manifest/run/snapshot context, and exported
> file paths, byte sizes and checksums. Native publication records table
> schemas and row counts; `read_result` does not revalidate them.

The following `read_notebook_source` wording retains its additional exact
source/cell identity, hash, language, frame and range checks. The other current
reader description in the contract's App boundary section already describes
manifest, source-scheme and checksummed-file validation accurately. Archived
attempt-1 receipts and its patch retain their original historical wording.

## Exact identities

| Item | SHA256 / identity |
|---|---|
| Corrected release contract | `c0429c9dfc20e9057698a660d8b963303c14bb8e9baf5316d11405331d9c4c61` |
| Original contract, retained by attempt 1 | `6413deee39590457a02fe6682453c8f11cf4313663a57978a0658f1de1bb790f` |
| Reused verified native package | `c5f36a0f99213b570c1fe862561f3bb8882f40936c0afe6565f46ceca1878d48` |
| Unchanged package sidecar | `187b64ec5da7b4fe5a3120df233b63983e62fb6c811c4ff77339a572f92b069d` |
| Unchanged extractor | `source-sha256-v1:6ff57778ccc62a9cbd78f39549836fd9a7c91f55c3f94830443b239e8a17f2c2` |

Package and build outputs stay at
`/private/tmp/spur-notebook-u5-bd-c1v3-final`; this attempt does not rebuild,
repackage or modify them. The fresh standalone output is
`/private/tmp/spur-notebook-u5-bd-c1v3-attempt2-native`.
[native/archive.json](native/archive.json) binds its retained fixture, native
documents, schemas/counts, manifests and raw child streams to their originals.
[native/evidence.json](native/evidence.json) binds the corrected contract hash,
unchanged acceptance/consumer hashes, source fingerprint and package/sidecar.

## New verification

Python is `/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python`;
package and native engine both report DuckDB **2.0.0**, platform **osx_arm64**.
All phases below execute on **HOST**. Every phase uses the exclusive
`capture.py PHASE COMMAND [ARG...]` wrapper in this directory. Each named
`.json` records exact argv, cwd, executable/native paths, explicit environment,
source/artifact hashes, raw exit, timestamps and stdout/stderr hashes; raw
streams are its `.stdout` / `.stderr` siblings. `SPUR_REMOTE=0` labels host
execution; no Rust command runs in this attempt.

| Receipt | Exact scope | Exit / result |
|---|---|---|
| [examples-extraction.json](examples-extraction.json) | Extract current README Python and contract SQL examples; compare with their original retained bytes | **0**, unchanged |
| [notebook-schema.json](notebook-schema.json) | Existing `test_exact_schemas_and_registry_docs` | **0**, 1 passed, no skips |
| [ordinary-signatures.json](ordinary-signatures.json) | Existing `test_existing_signatures_and_types_remain_usable` | **0**, 1 passed, no skips |
| [documented-python.json](documented-python.json) | Execute extracted public Python example against the verified package | **0**, seven function results |
| [documented-sql.json](documented-sql.json) | Execute extracted public SQL example against the verified package | **0**, seven statements |
| [native-acceptance.json](native-acceptance.json) | Unchanged standalone real-notebook consumer acceptance | **0**, all original assertions |
| [archive-native.json](archive-native.json) | Preserve this attempt's raw native streams and evidence | **0** |
| [verification.json](verification.json) | Receipt/source/package/history verification and old/current readback | **0** |

The standalone invocation is exactly:

```sh
/Users/kevintruong/.spur/jupyter/venv-duckdb-2.0/bin/python \
  docs/research/artifacts/2026-11-17-notebook-symbol-uplift-release/check_native_notebook.py \
  --release /private/tmp/spur-notebook-u5-bd-c1v3-final \
  --out /private/tmp/spur-notebook-u5-bd-c1v3-attempt2-native
```

The unchanged acceptance checks tracked real inputs and independent anchors,
46/24 symbols, 17/11 functions, 28 anchors, all 70 symbol names/source refs,
original cell IDs/hashes/UTF-8 ranges, same-document and duplicate-helper no-ops,
Rust `:dep` original coordinates and explicit unsupported forms. It retains
50 symbol changes and 10 cell events; all five schemas and registry descriptions
equal attempt 1. The source-grounded five offset-only slices explaining the
PoC's 55 versus production's 50 remain in `native/evidence.json.offset_only`.
Both immutable publications retain all 12 typed tables. Recorded child exits
are **0, 23, -15, 1, 0**; failed children preserve the old pointer/files and
are reaped. Original and subsequent healthy runs reopen successfully.

## Preservation and limits

[verification.stdout](verification.stdout) checks all **325** compiled source
inputs, every current native artifact, the package and sidecar, all **157**
prior-attempt files against their exact original commit, and the original
**281** historical evidence files plus historical package `28ddce83...` and
prior U3 artifacts. Old receipts retain their original documentation hash;
they are validated against `040dc7ba6` rather than relabeled with today's hash.
The attempt-1 verifier remains unchanged and is documentation-bound to its
historical contract; use this attempt's verifier for the corrected contract.

Previously verified **REMOTE 61 core + 4 adapter** and **HOST 223 native
Python, zero skips** are retained and checked as historical receipts, not rerun
for prose. All unsuccessful/remote/native receipts and native-names evidence
remain unchanged. The production diff is solely the contract paragraph; no
reader changes, new regression tests, frontend edits, deployment or AWS changes.

Public Rust, SQL v1 nested schemas and consumer signatures are unchanged in the
[contract](../../../../crates/spur-graph-duckdb/docs/release-contract.md#notebook-symbols-and-changes-v1).
The reader's schema/count validation limitation is now explicit. Existing
coordinate, preprocessing, coverage, anonymous-identity and caller-supplied
provenance limits still apply. Root owns independent actual notebook/runtime
review, final integration and issue closure.
