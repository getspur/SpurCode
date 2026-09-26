---
name: solve
description: >
  Use for constraint-shaped work: navigate the mathematical rule catalog with
  solve_rule_spec, verify or synthesize catalog models with solve_rules, derive
  constrained constants, prove an invariant or bound for all inputs, optimize
  competing preferences with Z3 Optimize or MaxSMT using weighted soft
  constraints or minimize/maximize objectives, diagnose conflicting policy
  rules, or find a branch-triggering counterexample with the generic solver.
  Also use when combining Jev intent interpretation with validated solver
  requests and Z3 results.
---

# solve - Catalog-First Constraint Workbench

Use Z3 as a workbench for evaluating declared rules and producing valid models.
The solver does not inspect a UI, runtime, or renderer. The agent supplies facts,
selects rules, and interprets the returned proof status.
When intent needs interpretation, Jev can propose a catalog request; it does
not validate that request or prove the encoded property. Keep interpretation,
validation, and solving as separate stages.

<HARD-GATE>
Before hand-encoding a domain rule, navigate the catalog with
`solve_rule_spec`. If an implemented rule applies, execute it with
`solve_rules`. Use `solve_constraints` only when the catalog has no suitable
rule. Before a generic solve, discover the typed language with
`solve_constraint_spec` and pass the exact request through
`solve_constraint_check`. Use `solve_smt` only when the typed constraint
surface cannot express the theory.

Never invent a multi-rule constant or treat `unknown` / `timeout` as `unsat`.
</HARD-GATE>

## Routing

| Need | Route |
|---|---|
| Discover whether a mathematical rule already exists | `solve_rule_spec` |
| Interpret natural-language intent into a candidate catalog request | `solve_rule_spec` → optional `jev_compile` → inspect bindings → `solve_rules` |
| Verify a complete domain model or fill bounded domain unknowns | `solve_rules` |
| Discover the generic request, variant, operator, limit, or example contract | `solve_constraint_spec` |
| Validate generic arguments without launching Z3 | `solve_constraint_check` |
| Execute a preflighted uncatalogued constant, invariant, policy, or branch condition | `solve_constraints` |
| Use arrays, strings, datatypes, quantifiers, shifts, or another unsupported theory | `solve_smt` |
| Reload a persisted result across delegation | `get_solve_result` |

Do not duplicate catalog formulas in prompts or in this skill. The registry is
the source of truth for available families, profiles, rules, examples,
authority, and solver encodings.

## Navigate The Rule Catalog

Use progressive disclosure. Load the smallest response that answers the next
decision.

1. Call `solve_rule_spec({})` for the bounded catalog summary.
2. Narrow with exactly one selector: `family`, `profile`, `rule_id`, or
   `primitive`.
3. Start with `include: "summary"`.
4. Load one detail only when needed: `valid_example`, `invalid_example`,
   `llm_encoding`, or `solver_encoding`. Use `all` only for a full audit.
5. Check capability status. Execute only implemented hard rules. Treat advisory
   or unsupported entries as guidance, not proof.
6. Build the family request from the returned inputs and call `solve_rules`.

Catalog IDs are versioned data. Never hard-code an exhaustive family or rule
list into agent policy; discover the current list on each new rule-shaped task.

## Collaborate With Jev

Use `jev_compile` when the user asks for Jev or natural-language intent needs
help selecting a catalog rule or mode. Otherwise skip it for an already
explicit, well-bound request. If the tool is unavailable, disclose that and
continue through the direct solver route when intent and required facts are clear.

The current MCP tool compiles **family requests only**, with one rule binding
per request. It does not run Z3. Generic compilation and repair helpers in the
Rust library are not additional MCP capabilities; do not invent a Jev generic
or repair tool. For an uncatalogued invariant, use the Generic Fallback below.

### Prepare And Inspect The Candidate

1. Navigate `solve_rule_spec` first. Read the matching rule's inputs and
   semantics; catalog examples may be illustrative projections, not complete
   executable requests.
2. Call `jev_compile` with `intent`, explicit `data`, and optionally a catalog
   `family_hint`. Use the catalog's family ID, not a guessed rule-ID prefix.
   The hint narrows routing; it does not override the intent.
   Put bindings at the top level of `data`: `subjects`, optional `parameters`,
   `scene` or `facts` as required by the family, and `unknowns`. Nested
   `data.rules[].subjects` is not a substitute for `data.subjects`.
   Use complete facts and no unknowns for verification, or explicit bounded
   unknowns plus known facts for synthesis. Jev does not infer missing geometry,
   normalize arbitrary payloads, or supply missing domain values for you.
3. Inspect the returned `gate`, ambiguity details, and provenance before using
   any request. For an open gate, check family, mode, rule ID, subjects,
   parameters, facts, and unknown bounds against the user's actual intent.
   One selected rule is not coverage of a multi-rule task: assemble all required
   bindings from the catalog before solving and record any changes to the
   candidate. Do not silently weaken rules or invent facts to obtain a pass.

### Confidence Is Not Proof

| Outcome | Next action | What it establishes |
|---|---|---|
| Jev gate open | Inspect the candidate, then submit the intended family request to `solve_rules` | Interpretation confidence only; neither request validity nor satisfiability |
| Jev gate blocked | Inspect the weakest decision and ambiguity; obtain missing evidence or clarify intent | No executable Jev request; not `unsat` or a failed rule |
| Solver rejects request validation | Correct the reported schema/binding problem from the catalog, then resubmit | No solver verdict yet |
| Solver returns a result | Interpret status with the selected tool and mode | A result for the exact encoded facts and constraints only |

`solve_rules` owns family validation and execution. Do not send a family
request to `solve_constraint_check`; that preflight is for the generic typed
language. An open Jev gate must never skip downstream validation or solving.

A blocked gate is not permission to lower the threshold, fabricate an open
request, or repeatedly rephrase until it passes. Recompile when new evidence
resolves the uncertainty. The current completeness question can reject valid
synthesis data because declared unknowns are not concrete. If intent, bindings,
and bounded domains are independently unambiguous, author a direct catalog
request and let `solve_rules` validate and solve it. Report this as a direct
solver path, not successful Jev compilation or an override of its gate. If
required facts or intent remain ambiguous, ask for clarification instead.

## Execute Catalog Rules

Choose the mode from the question, not from the status you hope to receive.

| Mode | Input contract | Success | Rejection |
|---|---|---|---|
| `verify` | Complete supplied model; no unknown declarations | `sat` + `pass` | `unsat` + `fail` |
| `synthesize` | Explicitly bounded unknowns plus known facts | `sat` + `solution` | `unsat` + `infeasible` |

For verification failure, use `rule_results` to attribute rejected bindings in
caller order. Preserve the flattened raw solver envelope for diagnostics.

`unknown`, `timeout`, `error`, and `ended` are inconclusive or operational
states. They are never pass, fail, feasibility, or impossibility proofs.

Do not apply the generic assert-negation convention to `solve_rules` verify.
The family compiler already owns its verification semantics.

## Generic Fallback

When no catalog rule expresses the actual invariant, do not infer the recursive
grammar from the flat execution schema. Its provider-compatible shape cannot
encode every tagged-union condition. Use this authoring loop:

1. Call `solve_constraint_spec({})` for the bounded language summary.
2. Narrow only unfamiliar entries with exactly one selector: `section`,
   `variable`, `expression`, or `operator`. Load a valid example when field
   placement or scalar type is uncertain.
3. Declare only necessary variables and prefer bounded domains.
4. Author every top-level constraint in the canonical wrapper:
   `{"id":"rule_name","expr":{...}}`. Put one named hard constraint per
   real rule.
5. Call `solve_constraint_check` with the exact intended execution arguments.
   On `-32602`, repair the field at `data.path` using `data.hint` and
   `data.example`, then check again. Never guess between similarly named fields;
   for example, a `kind=var` reference uses `name`, while `var` belongs to
   `kind=enum_label`.
6. Call `solve_constraints` only after preflight returns `valid: true`.
7. Establish hard feasibility first. Then add soft preferences or objectives,
   preflight the revised request, and solve again.
8. Test predicates, not one arbitrary model as a golden value.

The code-owned language catalog is the source of truth for current variants,
operators, arity, sorts, limits, and examples. Do not copy an exhaustive grammar
into prompts or agent policy.

### Generic status semantics

| Query shape | `sat` | `unsat` |
|---|---|---|
| Feasibility | Concrete valid model | Rules conflict or no model exists |
| Counterexample search | Concrete violating input | No violating input exists in the encoded domain |

To prove a custom property `P`, assert the counterexample `not(P)` under the
full input bounds. `unsat` proves `P` for that encoded domain; `sat` returns a
counterexample for a failing test.

For unsatisfiable named hard constraints, inspect `unsat_core`. Do not combine
core diagnosis with soft constraints or objectives in the same request.

## Preferences And Optimization

Any satisfiable model proves feasibility, not uniqueness or global optimality.
First obtain a hard-satisfiable baseline. Then choose the optimization encoding
by intent:

| Intent | Typed encoding |
|---|---|
| Rule must always hold | Named hard constraint |
| Preference may be violated at a cost | Soft constraint with a positive `weight`; repeat `group` only when preferences intentionally share one Z3 objective |
| Optimize a numeric or bit-vector expression | Explicit `minimize` / `maximize` objective |

Diagnostic soft `id` values identify results; they do not create objective
groups. Ungrouped soft constraints share one anonymous weighted MaxSMT
objective. Keep core diagnosis separate: do not combine unsat-core analysis with
soft constraints or explicit objectives in the same request.

Choose `objective_priority` from the question:

| Priority | Meaning and collection bound |
|---|---|
| `lex` | Default. Optimize objectives in declaration order and collect one solution. |
| `pareto` | Enumerate a solver-defined Pareto-front prefix, bounded by `max_solutions`. |
| `box` | Optimize generated objectives independently and collect one model per generated objective. |

Use the live tool schema for current numeric caps. Collection order is
solver-defined; compare solution sets unless the domain itself makes order
meaningful. Ratcheting remains a fallback when a preference cannot be expressed
by the typed objective surface.

### Retrieve Optimize Results

For a satisfiable typed optimization request, the top-level `model` is only the
first solution's compatibility view. Retrieve the complete result from
`optimization.solutions`:

- each solution's `model` is the optimized assignment for that point;
- `objectives[].value` is the expression's value in that model;
- `objectives[].bound` contains `kind` (`finite`, `infinite`, or `strict`) and
  lossless Z3 arithmetic text in `exact`;
- `soft_constraints` is in declaration order and reports `index`, optional `id`
  and `group`, effective `weight`, and `satisfied`;
- `groups` contains one aggregate per encountered group. Group rows use
  first-declaration order. An anonymous row appears only when ungrouped soft
  constraints exist;
- `termination` is `complete`, `solution_limit`, or `unknown`.

`solution_limit` means a Pareto prefix hit `max_solutions`, not that the frontier
is complete. A terminal `unknown` after at least one point leaves a partial
top-level `sat`; an initial `unknown` remains inconclusive. Never claim an
optimum or complete frontier from the top-level model alone.

Describe the result as feasible and optimized or ratcheted under the encoded
preferences. Claim uniqueness only when hard constraints force it or the
finite domain was exhaustively covered.

## Raw SMT Escape Hatch

Use `solve_smt` only when B-prime cannot express the required theory. The script
must remain within the allowlisted command and size guards, use fixed solver
arguments, and end in `check-sat`. Do not use raw SMT merely because it is
familiar.

Raw scripts that call `get-objectives` expose each exact bound, but omit `op` and `value`.
Z3's objectives response does not evaluate the expression in the returned model.
Prefer typed solves when the handoff needs operation identity, model values,
soft diagnostics, priority, or bounded-enumeration termination.

## Persistence And Handoff

For brain-to-worker handoff:

1. Solve with `persist: true` and record the returned `solve_id`.
2. Put the `solve_id`, selected catalog rule IDs, mode, and key interpretation
   in task context.
3. Reload with `get_solve_result`; never read `.spur/solver/` directly.
4. Read the returned `request` as the exact persisted solve input before
   interpreting the result or comparing it with the implementation.
5. Persisted results retain the complete `optimization` envelope; after reload,
   use the same `optimization.solutions`, diagnostics, bounds, and `termination`
   paths described above.
6. Treat the artifact as authoritative evidence, while still checking that the
   implementation matches the encoded facts.

When Jev participated, also retain its returned provenance (including model
version), gate/confidence and weakest decision, selected rule/mode, and any
candidate-to-execution changes. Keep the exact executed request and solver
status separate from Jev's interpretation; a `solve_id` does not by itself
preserve the Jev interaction. Never include credentials in the handoff.

## Proof Discipline

- `sat` is a model, not automatically a pass: interpretation depends on the
  selected tool and mode.
- `unsat` is a proof for the exact encoded query, not a process failure.
- `unknown` and `timeout` mean the solver did not establish an answer.
- A solver model complements tests; it does not replace RED-GREEN runtime
  coverage.
- Re-run the relevant solve after implementation when constants, bounds, or
  encoded policy changed.
- Do not infer UI hierarchy, visibility, units, or intent. Supply those facts
  explicitly before invoking catalog rules.

## Compact Workflow

```text
recognize constraint-shaped work
  -> navigate solve_rule_spec
  -> catalog match?
       -> intent interpretation needed or Jev requested? jev_compile
            -> open: inspect candidate and complete required bindings
            -> blocked: clarify, or independently author a direct request
       -> solve_rules validates and executes the intended family request
  -> otherwise solve_constraint_spec
  -> author canonical wrapped constraints
  -> solve_constraint_check until valid
  -> solve_constraints hard feasibility
  -> add preferences/objectives only after feasibility, then re-check
  -> unsupported theory only: solve_smt
  -> preserve raw status and mode semantics
  -> implement with tests
  -> re-solve the implemented policy
```
