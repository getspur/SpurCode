# Jev Layered-Config Graduation Implementation Plan

> **For SPUR orchestrator:** This plan is designed for `submit_plan(persist_as_epic=true)`.
> Each task becomes a beads issue with `spur:plan-task-id` and `spur:plan-id` labels.

**Source spec:** beads issue **bd-1i9zu** (approved design: `[jev] enabled` layered-config section; env stays as override; key stays env-only; worker exclusion invariant independent of toggle source)
**Design epic:** bd-bsgoh (closed; PR #55 shipped the env-gated v1)

**Goal:** Replace the env-only `SPUR_JEV_ENABLED` gate with `env SPUR_JEV_ENABLED=1 OR [jev] enabled=true` from the layered config, mirroring the `[skills] projection_mode` pattern exactly.

**Architecture:** One boolean config field flows through the existing layered loader (`spur_acp::config::load_layered`) into the brain-registry gate in `spur-core`. Worker exclusion and secret hygiene are untouched. TDD throughout; pre/post solve proves the precedence predicate.

**Worker:** codex (model gpt-5.6-sol, effort high). Build/test via `scripts/spur-cargo` only.

**Precedence spec (normative):**
```
jev_registered := env(SPUR_JEV_ENABLED == "1") OR load_layered(repo_root)?.jev.enabled   // config load failure => false
```
- Default (no env, no/absent `[jev]` section): NOT registered — zero behavior change for existing users.
- `JEV_API_KEY` remains env-only, never in layered TOML.

---

### Task 1: `[jev] enabled` config section + registry precedence + tests + docs

**Task ID:** `jev-cfg-1` (tracks beads issue bd-1i9zu)

**Files (complete list — nothing outside these):**
- Modify: `crates/spur-acp/src/config/mod.rs` — add `pub struct JevConfig { #[serde(default)] pub enabled: bool }` (impl Default → false); add `#[serde(default)] pub jev: JevConfig` to `SpurConfig` (line ~917 area, next to `pub skills: SkillsConfig` ~952); add inline tests beside the skills round-trip tests (~line 2171 pattern): `jev_section_round_trips` (`[jev]\nenabled = true` parse + serialize round-trip) and `jev_defaults_to_disabled_when_absent` (empty config, `[skills]`-only config, `SpurConfig::default()` all ⇒ enabled false)
- Modify: `crates/spur-core/src/mcp/mod.rs` — replace the env-only condition (current `if std::env::var("SPUR_JEV_ENABLED").as_deref() == Ok("1")`) with the precedence spec above; config read via `spur_acp::config::load_layered(repo_root)` — the function already receives `repo_root` (it passes it to `shared_solver_service`); a config load error must fall back to `false` (log at debug, never panic the registry)
- Modify: `crates/spur-core/tests/jev_mcp_registry.rs` — extend the existing `jev_compile_is_flagged_brain_only_solver_tool` with a config-driven leg using a temp repo root containing a layered config with `[jev]\nenabled = true` and NO env var: tool present; with `[jev]` absent/false and no env: absent; env=1 with config false: present (override); worker-exclusion assertion must run on the config-enabled path too (reuse the existing `JevFlagGuard`; note how the registry entry point under test receives the repo root — if `brain_tool_registry` does not accept one, use the entry point the mod.rs registry fn exposes for tests, following existing usage in this test file)
- Modify: `crates/spur-jev/README.md` — Environment section: document both channels with the precedence formula, config example (`[jev]\nenabled = true`), and the rule that `JEV_API_KEY` stays env-only

**Depends on:** none

**Acceptance Criteria:**
- [ ] Default behavior unchanged: no env + no `[jev]` ⇒ registry identical to pre-change (flag-off EXPECTED no-drift)
- [ ] Config-enabled (no env) registers `jev_compile` brain-side; worker tools still exclude it (test-enforced on this path)
- [ ] Env=1 overrides config=false; env config-load failure ⇒ false
- [ ] `scripts/spur-cargo test -p spur-acp` green incl. new round-trip/default tests; `scripts/spur-cargo test -p spur-core --test jev_mcp_registry` green; scoped clippy `-D warnings` clean for touched crates
- [ ] PRE/POST solve_ids recorded in a `[[spur-audit v1]]` comment on bd-1i9zu

**Suggested Worker:** codex (gpt-5.6-sol, high)

**Scope Boundary:**
- IN scope: the four files above, exactly
- OUT of scope: `crates/spur-jev` sources (no changes needed — the module and from_env are toggle-agnostic), the `/configure` TUI pane (explicitly deferred; separate decision), any docs outside the README section, any new MCP tool behavior
- Emit `scope_drift` immediately if any other file appears necessary

**Solve obligations (pre-then-post):** This is constraint-shaped (boolean precedence predicate).
- PRE (before implementing): `solve_constraints` with persist:true — bool vars `env_set, env_on, cfg_on, enabled`; hard named constraint `enabled_iff_spec: enabled == ((env_set ∧ env_on) ∨ cfg_on)`; expect `sat` (predicate is well-formed/feasible). Record solve_id.
- POST (after implementing): re-run PRE; then run the counterexample `not(enabled == ((env_set ∧ env_on) ∨ cfg_on))` over the same free booleans expecting `unsat` (for the finite boolean domain this is exhaustive: the spec predicate admits no assignment outside itself — guards against typos like OR/XOR mixups when transcribing into tests); then confirm the implemented condition in mod.rs reads exactly as the spec. Record both solve_ids in the audit.

**Implementation (TDD):**
- Step 1 — RED: add `jev_defaults_to_disabled_when_absent` + `jev_section_round_trips` (config/mod.rs) and the config-enabled registry leg (jev_mcp_registry.rs); run `scripts/spur-cargo test -p spur-acp` and `--test jev_mcp_registry` → FAIL (no JevConfig / config leg red)
- Step 2 — GREEN: minimal `JevConfig` + `SpurConfig.jev` field; registry precedence change; README
- Step 3 — full verification: both test binaries + `scripts/spur-cargo clippy -p spur-acp -p spur-core -- -D warnings`
- Step 4 — POST solves; audit comment with solve_ids on bd-1i9zu
- Step 5 — commit `feat(spur-jev): graduate SPUR_JEV_ENABLED to layered [jev] config section`

---

## Dependency DAG
Single task. Terminal on approval.

## Self-Review
1. Spec coverage: bd-1i9zu goal/constraints/acceptance all mapped (config field, precedence, key env-only documented, worker exclusion on config path, round-trip tests, README). `/configure` TUI surfacing explicitly deferred per issue. 2. No placeholders. 3. Types consistent (`JevConfig` referenced identically in mod.rs + registry). 4. DAG trivial. 5. File list complete — includes every file touched (lesson from 7 scope stops in the prior plan).
