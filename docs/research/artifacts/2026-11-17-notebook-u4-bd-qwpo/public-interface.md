# U4 public consumer interface

```python
def run_consumer(fixture, output, package, *, expected_extractor_id,
                 timeout=None, fault=None) -> dict: ...
def read_result(output, snapshot_id, *, run_id=None) -> dict: ...
def read_notebook_source(output, snapshot_id, blob_oid, source_ref,
                         *, run_id=None) -> str: ...
```

`run_consumer` returns completed consumer contract_version=2 and
notebook_contract_version=1. The fixture remains format_version=1 and
source_identity_scheme=sha256:utf8-v1. `read_result` returns
`{'snapshot_id': snapshot_id, 'manifest': manifest}` for consumer v1 or v2,
checking completed status, exact requested snapshot, declared contracts, coherent
immutable manifests, every table size/hash and the snapshot manifest hash.
Omit run_id to read the current pointer once; supply its 32-character UUID hex
to reopen a retained immutable run. New publication does not relabel an old run.

`read_notebook_source` loads no native extension. It requires consumer v2 and an
exact `(run, snapshot_id, blob_oid, notebook_path, extractor_id)` source match.
It checks document/cell SHA256s, ordinal, cell ID/identity quality, language,
decoded_code_cell_utf8 coordinate frame and original byte/line ranges. It
returns the selected original decoded-cell UTF8 substring. Wrong current or
historical context raises ValueError; it never substitutes another snapshot's
source. V1 results have no notebook source-ref contract and are rejected here.
Identical content explicitly present in two snapshots is valid in each one.

Two additive typed Parquet tables:

```text
notebook_explore(
  snapshot_id VARCHAR, occurrence_id VARCHAR, repository VARCHAR,
  commit_oid VARCHAR, parent_oid VARCHAR, blob_oid VARCHAR,
  logical_path VARCHAR, extractor_sha VARCHAR, status VARCHAR,
  document Document)

notebook_review(
  pair_id VARCHAR, base_snapshot_id VARCHAR, head_snapshot_id VARCHAR,
  logical_path VARCHAR, old_path VARCHAR, kind VARCHAR,
  old_blob_oid VARCHAR, new_blob_oid VARCHAR,
  base_repository VARCHAR, base_commit_oid VARCHAR, base_parent_oid VARCHAR,
  head_repository VARCHAR, head_commit_oid VARCHAR, head_parent_oid VARCHAR,
  extractor_sha VARCHAR, status VARCHAR, result Comparison)
```

Document/Comparison are the unchanged native notebook v1 STRUCT/LIST types from
the [U3 interface](../2026-11-17-notebook-u3-bd-lnhs-attempt2/public-interface.md).
Actual expanded schemas, including all nested numeric types and quoted fields,
are in each table's `schema` under `real_manifest.tables` in
[native-readback-final.stdout](native-readback-final.stdout). Tests compare
these payload schemas to the exact native definitions and reopened Parquet.

`notebook_explore.status` is native complete/partial/invalid, or null_input for
NULL source/document. Every cell remains inside document.cells, including
empty, imports-only, unsupported and non-code cells. Source NULL is not an
empty notebook. Cell metadata and diagnostics stay typed inside their owning
cell; opaque metadata alone uses metadata_json. `notebook_review.status`
retains the native comparison status and all symbol/cell/document change lists.
Before/after documents and refs retain their respective hashes and paths;
outer columns supply caller-frozen snapshot lineage. Either notebook path routes
a pair here. Ordinary pairs remain in the unchanged `review` schema; graph-only
`explore` retains its previous schema and all source occurrences.

Manifest snapshot coverage adds `notebook_status_counts`; extraction_complete
requires successful graph extraction and complete notebook coverage. Manifest
review_coverage retains previous fields and adds `notebook_pair_rows`,
`notebook_status_counts`, and `notebook_pairs_with_diagnostics`. The latter
includes parent and cell diagnostics on either side. Partial coverage remains
visible even for an unchanged document with zero edits.
