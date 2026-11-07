# G02 worker handoff

Canonical issue **bd-ode3**, runtime delegation **71911d279f084886**, source issue
bd-1zk7. Base: 0c6318bc619dad602ae11032cf45fd6239b46957. No subdelegation,
approval, close, merge or push occurred. Leave in_progress for ROOT POST/review.

The adapters reuse accepted typed validators and relational schemas so generic
transport preserves original evidence without changing storage or backends.

Production changes are exactly the three new providers files and one provider
module declaration; tests occupy generic_evidence_providers.rs. The compressed
G02.patch.gz is the complete unified diff of those five paths against the base.
Its uncompressed identity is recorded in diff-receipt.json.

Commits:

| Checkpoint | Commit |
| --- | --- |
| Tests first, actual collector fixtures | 5510f6460 |
| Original compile RED; existing-type import correction | c43d671f9 |
| Pure providers, static composition, typed query adapters | 52340df62 |
| Native replay and independent fixture provider instrumentation | ba323918a |

The initial RED had 54 unresolved errors including one missing TemporalIndex
import. After correcting that import, confirmed RED had exactly 53 errors, all
for the missing new provider module/APIs. No deliberately faulty stub was used.

## Verification

All heavy commands used scripts/spur-cargo, SPUR_REMOTE=1,
SPUR_NO_LOCAL_FALLBACK=1, SPUR_SYNC_FULL=1, and the existing builder
i-06f135acdd1181f53. Full sync is necessary because rust_evidence_contract
embeds root-document fixtures. Builder configuration, shared cache files,
locks and quotas were left to the existing wrapper; no local fallback occurred.

| Check | Actual outcome |
| --- | --- |
| Final generic_evidence_providers | 10 passed, 0 failed/ignored/filtered |
| Final macro_invocation_artifacts | 8 passed, 0 failed/ignored/filtered |
| Final rust_evidence_contract | 16 passed, 2 failed, 0 ignored/filtered |
| Remote check -p spur-graph --all-targets | exit 0; existing warnings retained |
| Local fmt -- --check | exit 0 |
| Full git diff --check | see diff-check.log.gz and raw-log-receipts.json |
| Native verification | 3 original providers, 11 independent mutation cases, 24 compiled source bindings |
| Historical ROOT PRE verification | expected fingerprint/counts/SAT+UNSAT pair, prospective only |

The full required three-suite command was run twice without exclusions. It
remains red on these two existing historical comparisons:

- d2_absence_empty_legacy_bytes_and_capability_encoding
- d2_entire_typed_envelope_hashes_and_rehashed_forgeries_reject

Both compare current manifest_version
37eee2e2e0d9213d4d9ff13ccbacae861859bd592305fb88dbe54e4e09024626
against the historical golden
bf1ad64f405c4cd5222120dad7b0d5461d0b7d2f52f9aeb0bbe79c07ae61d524.
Every version/query input to that identity is byte-identical to the approved
base. verify_baseline.py reproduces the existing SHA256 algorithm and checks
all 35 query inputs against git. No historical tests, fixture bytes or logs
were altered. Scope signal 4f0b12a5-82db-4e40-b29e-395fa7e97c89 records this
gap. ROOT must decide its handling; it is not reclassified as GREEN.

The first native collection attempt failed because request/config paths were
relative to the repository instead of Cargo's package working directory.
The file config was corrected to ../../docs/...; the raw failed log remains.
There were zero observed infrastructure failures. Warnings in untouched MCP,
Lance and overlay benchmark code remain in raw logs.

## Native replay packet

- native-collected.json.gz: original two built-in exports used to author mutations.
- collection-request.json: exact initial source-byte request.
- derive_mutations.py: independently mutates the actual exports, computes no BLAKE3.
- native-request.json: actual final request with 11 supplied adversarial cases.
- native-config.toml: Cargo FILE configuration forwarding env.G02_EVIDENCE_REQUEST.
- native-replayed.json.gz: final native outputs for Rust, legacy macros and the
  explicitly test-injected TestCounter, plus every supplied mutation.
- native-verification.json / verify_packet.py: observed assertions and replay authentication.
- raw-log-receipts.json: complete command exits, original raw lengths/SHA256s.

Each output includes actual graph/typed originals, extension tables, full typed
decodes, registry/stored/saved-source results, original source bytes and native
Git OIDs, compact typed/generic checksum preimages and native digests, compiler,
Cargo, executable and config identity, and compile-time source SHA256+lengths.
Legacy rehash sets are supplied mutated original records solely for native
checksum computation; validators still receive the mutated extension and real
graph/bytes. No prewritten result JSON or invented collector/cache events exist.

Observed boundaries:

| Mutation | Integrity | Stored | Saved source |
| --- | --- | --- | --- |
| Rust call text, independently rehashed | pass | pass | reject |
| Rust owner, independently rehashed | pass | reject | reject |
| Rust source bytes, native OID recomputed | pass | pass | reject |
| Macro raw path, independently rehashed | pass | pass | reject |
| Macro owner, independently rehashed | pass | reject | reject |
| Macro source bytes, native OID recomputed | pass | pass | reject |
| Forged original_format metadata | pass | reject | reject |

Unknown provider returns UnknownProvider/domain Unknown after integrity.
Both supported IDs with future schemas return UnsupportedSchema/domain Unknown.
Malformed opaque transport fails integrity first. TestCounter has one integer
per authoritative fixture source, including independent.py; its registered
native result validates, while the same extension is UnknownProvider in the
production registry. Duplicate registry construction rejects. Rust captured
empty, complete and Unknown lanes and full origin inventories round-trip;
macro lengths remain None with authenticated original_format metadata.

The merged final-suite log contains one Cargo stderr `Running tests/...` line
interleaved inside the large native stdout JSON. The original raw log is
preserved unchanged in native-replay.log.gz. native-extraction.json records
the exact foreign line, byte offset and SHA256 removed when extracting the
JSON record; verify_packet.py reproduces this unambiguous extraction and
authenticates the resulting packet. No native JSON fields are rewritten.

Authenticate this packet without rebuilding:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/worker/verify_packet.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/worker/verify_baseline.py
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/root-pre/verify.py
```

Native replay through the same public APIs (one target avoids interleaving a
later Cargo target's stderr):

```sh
SPUR_REMOTE=1 SPUR_NO_LOCAL_FALLBACK=1 SPUR_SYNC_FULL=1 scripts/spur-cargo test --config docs/research/artifacts/2026-11-07-generic-graph-evidence/G02/worker/native-config.toml -p spur-graph --test generic_evidence_providers -- --nocapture
```

ROOT should copy the request/config into a fresh evidence directory before
authoring additional cases. The request schema and optional native legacy
rehash inputs are in the integration test; cases use their supplied graphs,
extensions and source bytes directly. Worker artifacts remain immutable.

## PRE and remaining acceptance

Canonical INTENT d1cf5817-9660-43b4-bfcf-98cd5f848ccf persisted/replayed as
comment 4581. TDD checkpoint f149e302-731b-4c24-977f-78802286854e is comment
4586. Both current G02 solver receipts were reloaded with pin omitted and their
full envelopes equaled root-pre/proof.json.gz; pre-reloaded.json.gz preserves
the objects. Historical PRE was re-authenticated after edits without
--current-source; current native source identities are separately bound.

Fresh actual-output Jev POST, independent native review, handling of the two
historical golden failures, and local integration remain with ROOT. No G03 or
later stage is implemented or approved. No generic disk/backend/cache/runtime
acceptance is claimed.
