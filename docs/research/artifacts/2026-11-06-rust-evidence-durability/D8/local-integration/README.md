# D8 local main integration

Canonical repair `bd-3eyg` was approved after ROOT's fresh native replay and
persisted POST. Plan `ad663d7c-9434-46f1-ac9c-63b95c499a81` integrated through
`spur/plan-merge-ad663d7c-9434-46f1-ac9c-63b95c499a81-aa0589c35a0c491690a6ff413c80ab52`.
ROOT joined that lineage to its acceptance commit without changing any file
bytes. Local main then fast-forwarded from `98d73e8f1` to committed source epoch
`c611b67f5131727bf0dcc259f96f48598e889b2e`.

The committed-epoch verifier rederived POST fingerprint
`2c96923bc95d01d6ccfa18cdf580721b12fbf4d3953c1bf2ec1f48e45251618b`
against that exact Git revision: 2,620 bindings and nine independent predicates.
The 18 fresh ROOT native invocations and required worker checks are recorded
in `D8/ROOT-acceptance/` and `D8/empty-capability-repair/`.

All five existing user-edited tracked files retained their exact bytes. All
previous untracked paths remained present, and the main index remained empty.
The current working tree's preserved ACP edits are distinct from the committed
native acceptance cohort; they were neither overwritten nor claimed as tested
by this POST. No remote push or PR was created.

This final documentation commit changes only this integration record; the
reviewed source, native inputs and proof bindings remain unchanged. The known
macro fixture failure `bd-xb5p` remains explicit. D9's MCP capability/cache
identity and D10's fresh Rust notebook are the remaining planned phases.
