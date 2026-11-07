# D7B retry: footer statistics repair

Canonical issue `bd-2bur`, delegation `a0fcbcae99f14159`, plan `22f47352-bb77-4b70-b764-3188bd8562aa`. Source checkpoint: `f0a8947c6ed4f266d37be247fdedf5f268a2077c`. ROOT owns fresh actual-output Jev POST, review and integration. The canonical issue stays in_progress.

The shared evidence checksum binds decoded rows but excludes Parquet footer min/max statistics. The previous query reader trusted those statistics to exclude row groups after validating the complete envelope, allowing a filtered query to silently lose validated rows. The repair removes that exclusion step for the shared source/body/site/declaration reader and retains the Arrow predicate over actual file-path values in immutable compressed bytes. Full validation, whole-file groups, whole referenced origins, public APIs, owner/source context and frozen IO behavior remain intact. Scanning path columns can require more work than trusting footer statistics; no performance quota or measured speed claim is introduced.

Recovery fast-forwarded to `7b23b85fd144c62cc5f97d7d254963542838e304`, preserving normalized attempt 1, ROOT tests and protected authored history. `commits.json` records those references. New assertions were committed at `89301ebd4` before their fix at `f0a8947c6`. The original eight-test file remains SHA-256 `09ffb8a2c2beb09a116dc5e9411a6777cdf4bd2b3751aae2e8c967473eae779e`; the existing six-test D7B file is unchanged.

The fresh rework PRE was verified before edits: fingerprint `fe093c69e5b62d397360453a9060c48dfc549e1f029f8c91c240020fd9360cf6`, 2,489 bindings and nine independent predicates. All three complete live envelopes exactly matched the archive: `sol_0758615e27eb4b72` SAT/pass, `sol_5c4156dbf86442e2` SAT/pass, `sol_edd2d7acc44f4c86` UNSAT/fail. They are preserved in `pre-live.json.gz`. No consumed source-bound derivation was rerun after implementation changed.

| Executed verification | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: |
| ROOT regression before repair | 0 | 1 | 0 | 0 |
| New nonzero regressions before repair | 0 | 3 | 0 | 0 |
| Original query target after repair | 8 | 0 | 0 | 0 |
| Existing D7B target | 6 | 0 | 0 | 0 |
| ROOT query target | 1 | 0 | 0 | 0 |
| New nonzero query target | 3 | 0 | 0 | 0 |
| Query-client parity | 7 | 0 | 0 | 0 |
| Macro target | 13 | 1 | 0 | 0 |
| Query-client command, aggregate | 26 | 0 | 0 | 1,432 |
| Fresh native replay, 11 invocations | 13 | 0 | 0 | 631 |

The query-client command visited 118 targets; the library selection passed 26 with 566 filtered, and other targets executed zero tests. Graph all-target check, scoped local fmt check and complete diff checks exited zero. Complete raw logs, exact commands, environments and source checks are retained. The known macro failure is `parquet_macro_queries_reject_missing_source_manifest_rows`: stored validation rejects its stale graph content hash before the query. This retry independently revalidated all 803 accepted predecessor inputs against Git `deef4ad0a` and the preserved native identity/log. That baseline is separate from bd-16eq; no fixture or assertion was changed.

The first broad query-client attempt exited 101 during compilation because the default optimized remote sync removed six documentation fixtures referenced by include_str. Its log and command are preserved separately. The retry used the wrapper's existing `SPUR_SYNC_FULL=1` option and passed; all-target check used the same option. Risk signal `fdd0ae52-fd06-49b0-a014-c39902c3025d` records this. No infrastructure was edited. All builds used the prescribed wrapper, remote execution, no local fallback, disabled fresh-output capture, the bound cloud helper and existing rsync timeout 180. Logs stayed outside checkout until runs ended; source stayed immutable during uploads and builds. No shared targets were copied, jobs terminated or locks removed.

`source-bindings.json` binds 806 graph/config inputs; `restored-fixture-bindings.json` adds the six existing documentation fixtures. Native identity records verify actual builder bytes, config, toolchain executables and ELF hashes. Initial source records retained an inherited attempt-1 cloud-helper descriptor; it was historical metadata. `build-helper-identity.json` and the compressed helper snapshot bind the current prescribed helper separately, and the replay source record uses that observed identity.

`nonzero-red-packets.tar.gz` preserves original and footer-mutated native artifacts, raw source bytes, both filters, row identities and actual omission results for all three nonzero tables. `native-packet.tar.gz` preserves fresh complete native artifacts and results: ROOT's zero-source regression, nonzero body/site/declaration cases, all four footer-mutated tables, path-only and matching-OID selections, filtered-out source/body corruption, wrong-existing OID rejection, and the default lifecycle/Unknown/legacy/empty profiles. Every invocation verifies source and executable bytes before and after execution. The first exporter stopped after four successful native invocations because its IO-log parser assumed an unprefixed line; the raw incomplete packet, scripts and transport error are retained. The corrected parser exported all 11 invocations into a fresh path without source changes.

Fresh IO counters were before [0,0], frozen [5,23988], after replacement/source edit [5,23988], after a separate corrupt cold client [10,24023], and after deletion/existing-context query [10,24023]. Existing contexts therefore performed zero additional evidence-file reads. The cold client rejected corrupt bytes. `post-candidates.json` contains actual ordered outputs and exact POST candidates. ROOT's fresh repaired projection equals the original PRE control exactly, SHA-256 `d1982823bbdafd3d75f36bef5951a311e55ec90b096e1a13711ad52ec2f41dcf`.

Verify the packet:

```sh
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7B/worker-evidence/attempt2/verify.py
```

ROOT can invoke the unchanged arbitrary-input native hook through the refreshed driver:

```sh
python3 docs/research/artifacts/2026-11-06-rust-evidence-durability/D7B/worker-evidence/attempt2/replay.py \
  --instance i-026b6f74eb8d53f5e \
  --request /absolute/remote/request.json \
  --remote-output /absolute/remote/fresh-output \
  --capture /absolute/local/capture
```

Requests contain `artifact`, `disk_dir`, `filters`, and optional `before`/`after` mutation arrays. Supply an independently footer-mutated disk directory directly. Existing operations support string replacement, rehashed replacement, writes and deletion. The hook copies supplied disk inputs, exports actual results and permits independent negative cases. Omit the request for the native fixture/lifecycle packet; use `--io` for the IO replay. Driver checks bind source and executable hashes. Rebuild/rebind if the recorded builder or executable is unavailable.

`repair.patch.gz` is the retry's unified source/test diff; `full-D7.patch.gz` includes the complete source/test seam from dispatch base. Stored integrity validation does not authenticate mutable current syntax, establish call-target correctness or prove complete panic detection. This packet supplies concrete regression/native evidence; fresh ROOT POST and approval remain required. D8–D10, close, merge and push are outside this worker's completed scope. Entry audit was persisted/replayed as comment 4242; fresh completion audit receipts accompany the final evidence.
