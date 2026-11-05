# M4 current entry gate: bounded batches and local acceptance

Task `bounded-batches-local-acceptance`, **bd-2hgz**, plan
`dc9cc6fb-d54a-4429-a231-1f7f3425f7fd`; requested routing remains
**codex / profile codex / gpt-6.1-sol / xhigh**. This PRE is provided before
M3 approval and before any M4 production edit. No repeated user approval is needed.

Read the [root M3 review](../root-M3-review/README.md), then run:

```bash
python3 -B docs/research/artifacts/2026-11-05-embedding-sol/M4-pre/verify_pre.py
```

Reload and inspect all five receipts: `sol_133bceec77af45ee`,
`sol_d0d8dfba4e404a7f`, `sol_9e1e920117a3421e` (SAT),
`sol_3dcea2ba40834e56`, `sol_73220cb2afd34dab` (expected UNSAT).
`proof.json.gz` preserves complete requests, SMT lowerings, responses and reloads.
Jev gates opened at .97/.98 and facts, rule bindings, verify mode and no-unknowns
were inspected. Independent validation covers 11 compound catalog predicates,
including 339 numeric comparisons for the reference chunks and 339 for the
one-byte-short control. The other control rejects partial success.

The 51-file source fingerprint is
`dd5361615d064068fe6a06c1d57d6c5858f5a76da83ffdb45f14d9b4d75302ba`.
Current `EmbeddingModelHandle::embed` ignores its batch parameter and forwards
all inputs as one request. `embed_cancellable` similarly forwards the whole
input. Graph section/symbol paths split by `SectionEmbeddingOptions.batch_size`,
whose positive override is not capped by wire policy. Analyst in-process
compatibility passes arbitrary vectors through this handle; owned query paths
currently send single texts. The 4096-byte section-body eligibility threshold
does not bound titles, composed section text, symbols or queries.

`reference.json` records **19 finite cases / 113 valid chunks**, covering 63/64/65
rows, caller batches 1/2/128, empty inputs, explicit zero batch, empty text,
ASCII/Unicode exact and excessive per-text sizes, an independent total cap,
exact and one-byte-short JSON frames, escaped control characters and oversized
individual frames. Caps are derived from inherited limits or actual serialized
payload lengths; no new production numeric defaults are introduced. Python's
compact UTF-8 request encoding is cross-checked with the freshly executed Rust
writer's worst-case default request: **25,168,231 body bytes**, with one byte less
rejected and zero wire bytes published. M4 must add actual Rust boundary
assertions before implementation; this finite reference is not an implemented
batcher, runtime acceptance or universal proof.

Implement a reusable bounded admission/planning path in the neutral crate and
consume it in the graph model handle, or an equivalent minimal arrangement.
Use the validated shared policy. Every nonempty chunk must obey requested row
size, protocol row cap, UTF-8 per-text and total sizes, and the actual compact
serialized request body cap (the four-byte prefix is outside the body cap).
Preserve every input byte, index, order, prefix and model identity. Use checked
arithmetic; reserve actual correlated envelope overhead or a source-derived
safe upper bound such as maximum u64 request ID length. Do not invent a fixed
overhead allowance or infer JSON size from raw text bytes. Oversized individual
requests return a typed error without truncation or an infinite split loop.

Preserve existing graph option-zero normalization to default64. A new explicit
batch API may reject zero; do not change established configuration semantics.
An empty logical input returns an empty result without an Embed request;
existing high-level disabled/ineligible checks must avoid helper launch.
Keep the low-level single-frame API strict unless a justified minimal adapter
is needed. No owner State/Gate/job/publication/native/model redesign is authorized.

Use one effective caller deadline across chunks, cancellation before/between
requests and the existing completion/close gates. Reuse the supervised helper;
validate correlation, identity, exact rows/dimensions and finite values for
every reply. If a later chunk fails, cancel/close wins, or a reply is invalid,
do not return accumulated rows as a successful complete logical batch. This
does not turn the existing outer graph per-row `Option`/progress semantics into
a whole-build transaction. Preserve already committed results and explicit
restart policy semantics.

Required TDD: commit tests, observe runtime assertion RED on current behavior,
then minimal fix/GREEN. Include exact frame/UTF-8/row boundaries, oversized
individuals, second-chunk failure, wrong generation/model/row count/dimensions,
nonfinite data, cancellation and closure. Export actual request shapes,
serialized sizes, ordered outputs, failures, generation/identity observations,
helper reuse and cleanup results for ROOT's fresh POST. Add source-grounded
supplemental PRE via root before introducing new bounds or unrelated changes.

This last local implementation node must also close the original shared-policy
acceptance: common graph/analyst defaults, explicit overrides, compatible
sibling/absolute discovery, child cache, preserved Off/Auto/Sidecar and disabled
behavior, checked batching, ordinary separately built helper in a private
temporary layout, actual finite output and checked close. Use existing pinned
three-model references and tolerance1e-4 where real inference is compared;
record exact features, model/runtime/source identities, commands and exclusions.
ROOT must evaluate the original complete stage-A requirements POST and supply
shipped-packaging PRE before approval. Packaging and final three-model
performance/containment/fresh Notebook Rust acceptance remain later gates.

Use the current canonical dispatch issue/delegation, authorized INTENT plus
identical replay, MCP progress heartbeats and `scripts/spur-cargo` under the
approved local environment. ROOT has no Cargo writer running at release.
Preserve main, user files, notebooks, external source, graph/CURRENT and shared
model caches; use private copies. No installation overwrite, global env change,
service restart, push or publication. Leave issue in progress for root review.
