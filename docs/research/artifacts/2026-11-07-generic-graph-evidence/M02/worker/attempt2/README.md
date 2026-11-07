# M02 attempt 2 evidence

Recovered f161471f542a8aadafffb00ac29ae195db82dc3c via the required fast-forward from 4a411dbd24f306a27b48b7e11b2ab381b7e39797. Attempt-1 and ROOT evidence remain immutable. Historical entry verifier ran once at recovery; current-source PRE passed. Both complete live envelopes exactly match archived reloaded values in root-pre/proof.json.gz. See entry.json.gz.

Canonical issue bd-1zi2 is in_progress, assigned to spur-worker-f567f55354a44a99. ROOT INTENT 4706 and worker INTENT 4716 establish authorization; identical worker replay confirmed persistence. ACTION/runtime audit 4717 and baseline-risk audit 4722 are durable. Exact requests/responses are archived.

Unmodified remote core baseline: 44 passed, 5 failed, 0 ignored/measured/filtered, exit101. Three real-tool fixture failures report tuple owner absent or ambiguous; two freshness assertions return fresh instead of not_needed. See baseline-failures.json and full compressed log. Source/HEAD remained stable. Earlier sandbox attempt exited200 before Rust because EC2 was unreachable; network-enabled retry reached AWS, cold-started configured builder, uploaded full fixture context and ran native tests. No local fallback or VM/token/target/quota repair.

run.py records exact command/env, timestamps, source SHA256s, HEAD and complete lossless compressed logs. observe.py and compare_native.py are prepared, unexecuted observation tooling at this checkpoint; they do not establish native conservation. No source migration or POST is claimed at this checkpoint. Full suite baseline RED and open M01 analyst risk bd-16eq are retained for ROOT review.
