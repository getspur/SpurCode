<!--
Thanks for contributing to SpurCode! PRs from outside the core team are welcome.

CONTRIBUTING.md documents the test tiers and conventions in detail.
-->

## What does this change do?

<!-- Summarize the change and why. Link any related issues with "Closes #N". -->

## Checklist

- [ ] The change is described above and motivation is clear
- [ ] `scripts/spur-cargo fmt --all` was run
- [ ] `scripts/spur-cargo clippy --workspace -- -D warnings` passes locally
- [ ] Tests were added or updated to cover the change
  - [ ] Default `cargo test` invocations stay fast (annotate external/slow
        tests with `#[ignore = "requires ...; run with --ignored"]`)
- [ ] No secrets, credentials, or private data are introduced (also checked
      by the `vendor-leak-check` workflow)

## Notes for reviewers

<!-- Anything non-obvious: design tradeoffs, invariants, or follow-ups. -->
