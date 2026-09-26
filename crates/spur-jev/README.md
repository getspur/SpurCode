# spur-jev

`spur-jev` is SPUR's brain-side Jev pre-compiler. It turns natural-language
intent plus structured scene/fact data into a typed solver request; it never
executes the request or invokes Z3 itself.

## Pipeline

1. The brain registry builds a stable `CatalogSnapshot` from the executable
   `spur-solver` rule manifest.
2. `jev_compile` builds a deterministic, evidence-complete routing battery from
   `intent`, `data`, the optional exact `family_hint`, and that snapshot.
3. `HttpJevTransport` evaluates the battery and records the exact served model
   version and every typed answer.
4. The weakest-link gate considers both routing/mode confidence and the
   `data_complete` noul. A blocked gate returns `ambiguity` and no request.
5. An open gate deterministically compiles the answers and returns `request`
   plus redacted `provenance`.
6. The caller must cross the existing solver validation boundary before any
   execution:
   - family request: inspect with `solve_rule_spec`, then call `solve_rules`;
   - generic B-prime request: call `solve_constraint_check`, then
     `solve_constraints`.

The request returned by `jev_compile` must be consumed **only** through
`solve_rule_spec`, `solve_constraint_check`, `solve_rules`, or
`solve_constraints`, as appropriate for its shape. Never execute it directly,
feed it to raw `solve_smt`, or treat Jev output as proof.

## Invariants

1. **Pre-compiler purity.** `spur-jev` constructs JSON requests but does not
   invoke Z3, decide satisfiability, or bypass a family compiler or B-prime
   validation.
2. **No silent low-confidence solves.** Every decision that shapes compilation
   participates in the weakest-link gate. A value below `0.60` blocks and
   returns an ambiguity report instead of a request.
3. **Deterministic battery.** Identical intent, canonicalized data, catalog
   snapshot, and language version produce byte-identical question content and
   order. Family ownership comes from catalog metadata, never a rule-ID prefix.
4. **Provenance completeness.** Every successful or blocked response includes
   the exact served model version, all typed answers, every gated confidence,
   the weakest value, and the gate outcome. Credentials and headers are never
   included.

## Environment

- `jev_compile` is added to the brain MCP registry when either
  `SPUR_JEV_ENABLED=1` or the effective layered config enables Jev. In other
  words, `jev_registered = (SPUR_JEV_ENABLED == "1") OR jev.enabled`. The env
  var is an explicit per-process override; any other value falls through to
  config. The worker registry never exposes this tool.

  ```toml
  [jev]
  enabled = true
  ```

- `JEV_API_KEY` enables the live HTTPS transport to TypeSafe System One. It is
  env-only and must never be stored in layered TOML. When the brain tool is
  enabled without a non-empty key, calls return a typed
  `solver_unavailable`-style MCP error; there is no network fallback.
- `SPUR_Z3_BIN` belongs to the downstream solver service, not `spur-jev`. It is
  relevant only after an open-gate request is handed to a solver tool.

## Recorded POC evidence

The implementation is grounded by committed, replayable evidence:

- `scripts/jev_poc_probe.py`
- `scripts/jev_poc_probe2.py`
- `scripts/jev_poc_results/phase1_summary.json`
- `scripts/jev_poc_results/phase2_report.json`
- `crates/spur-jev/tests/fixtures/recorded_cases.json`

The mock MCP replay uses the recorded valid containment case (`child.x = 76`,
`child.width = 24`, `parent.width = 100`) and the recorded served model
`jev-1.13.0`; the boundary test expects solver outcome `pass`.
