# U4 attempt 1 root review

Request changes for two Important/P2 routing/readback gaps, confirmed by three private native regression probes. All 19 existing consumer/fault tests pass independently. Receipt/source identity checks, 325 native inputs, 37 immutable published files, exact schemas and actual archived v1 readback also pass.

1. Missing absent-side paths are accepted by fixture validation. An ordinary A pair with old_path=None and D pair with logical_path=None publish pair_rows=2 but zero review rows because the new SQL routing predicate evaluates to NULL. A notebook D pair with logical_path=None routes to the notebook API, receives a NULL result and then aborts the child at result['contract_version']. Every accepted pair must survive routing with explicit, documented absence handling.
2. A valid review-only notebook fixture with lineage=[] publishes native before/after source refs, but read_notebook_source only searches occurrence-based notebook_explore. Opening its valid head reference raises no matching snapshot source. Published review refs must remain usable without an Explore occurrence, retaining the exact run/snapshot/blob/path/extractor guards.

The three root regression tests failed meaningfully with exit 1: two ordinary rows lost, notebook deletion child TypeError, and unresolvable published review reference. reproducer.patch is a zero-context test-only patch against the reviewed HEAD. The isolated scratch tests were restored to their exact committed bytes afterward.

Independent reviewer bd-1g1u confirmed the NULL-routing regression and related notebook NULL dereference; root confirmed the additional review-reference witness natively. No scope expansion is required: both fixes belong in the existing consumer/test files. Preserve all typed payloads, historical v1 behavior, source identity scheme, immutable publication and child/failure guarantees.

See review.json for exact commands/environment and captured outputs. Native package ed42b39b and historical 28ddce83 remain unchanged. Frozen 46/24 symbols, 17/11 functions, all 28 anchors and 70 source refs pass in existing tests. Original TDD commits b359a4123/0342d3f1b precede 8427e33e9 and remain preserved. Final release is gated on these corrections.
