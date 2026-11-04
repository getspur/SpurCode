# S2 interim evidence — bd-14ak

Status: test-first API compile RED; supervisor implementation and root actual-output POST remain pending. The fixture is a protocol fault injector, not inference acceptance.

- Dispatch: `spur:delegation-id:01a645484d964cae`, issue `bd-14ak`, observed `in_progress`.
- Protocol candidate `2fd3ec7b842dbf0e3138eb0fdfd3328e2b65feb6`; `protocol.rs` SHA-256 `ac27917adecafbb1290a99fd473811d1142f8a0c5ea50d6b4444aa5c35759097`.
- Imported S1 approval evidence from `0c1f57a3441f1ebd106bd69ec10af6321d7f215e`, and S2 root PRE files from `4ab61cf53f60eb2d021cb9f09ceae652de113f9d` only in the authorized paths. The PRE archive SHA-256 is `c126186e9068cd5571bd400e829f9f568874f730d9c4c1fb1fa6055227097162`.
- Root's three Jev interpretations were open; all nine solver IDs in `root-pre.json` were reloaded through `get_solve_result`. Their complete lowered requests and statuses matched the archived reloads. These are prospective bounded policy checks, not runtime evidence.
- Test commits: `5f7d80972` and `efdd1ea24`. Command: `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo test -p spur-embedding --features test-support --test supervisor`. `red.log` records exit 101, unresolved client API imports; additional inference errors follow from the missing API. This is compile RED, not an assertion RED.
- Fixture-only compile check: `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target scripts/spur-cargo check -p spur-embedding --features test-support --bin embedding-fault-worker` exited 0; this did not execute the fixture. See `fixture-check.log`.
- A further root PRE was requested for queue capacity, restart budget, deadlines, cancellation and teardown sequencing before production implementation. No product policy defaults have been selected.

No production implementation, GREEN result, actual child status trace, POST solver evidence, or completion audit is claimed here.
