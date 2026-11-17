# Rebuild and Explore experience audit evidence

[Detailed report](../../2026-11-17-spur-graph-explore-experience-audit.md).

- `release/`: successful clean-source rebuild, 61+4 remote Rust and 223 native
  Python tests, package verification, final source probe and raw command logs.
- `setup-attempt-1/`: retained pre-compilation helper-discovery failure.
- `audit.json`: Notebook MCP sources/outputs and live provider failures.
- `native-acceptance/`: standalone real-notebook acceptance evidence.
- `app-source/`: captured notebook source and current provider/manifest/skill.
- Screenshots and `spur-explore-audit-resolution-exact.json`: actual render and
  exact SQL evidence of inconsistent primary target resolution.

New unsigned native package: `.spur/artifacts/explore-audit-bd-2bhj-native/`.
SHA256 `41f5e65ad778a4e90d513ef5dc2f5065d0f45629968a20a231ece962e9bf627f`.

The release checks pass. Adversarial ordinary duplicate-definition review fails
semantically and is tracked as bd-1gyj. App provider/render/state findings are
separate from native build readiness. No production source fix was made.
