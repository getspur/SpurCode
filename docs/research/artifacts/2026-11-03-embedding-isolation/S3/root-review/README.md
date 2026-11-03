# S3 root review: initialization error framing remains red

Candidate `557f4983c4c25ce03510bd56af51c4ba77d0c066`, delegation
`8127590442aa42f6`, canonical issue `bd-2hdm` is not approved. S4 stays gated.
Root changed no production code. Test-first regression commit:
`e1ed254750484376ba450886f16d6c834c600d72`.

The existing four native/protocol tests pass independently on macOS ARM64,
including real ONNX inference, retained-output refusal, checked native close,
active work and signal termination. The normal worker also builds. The ten
worker source hashes and both inherited executable hashes matched before the
root rebuild; `inherited-checks.json` records that check. Existing native
behavior is worth preserving while correcting the protocol seam.

Two added tests compiled and failed at runtime, exit 101, zero passed/two
failed. A valid Hello is sent with an intentionally unusable native cache
path. At caller error-message limits 1 and 256 bytes, the helper prints
`RuntimeError::Initialization` to stderr and terminates with exit code 1.
The response reader sees UnexpectedEof, with no correlated Error frame.
Raw Unix wait status is 256 (exit 1), not a native abort. Root separately
rebuilt the normal worker without test support and reproduced both missing
frames; `normal-startup-failures.json` records stdout bytes and stderr.

`main::run` calls `RuntimeOwner::start` before `serve` reads Hello. The `?`
therefore returns a boxed Rust error from main instead of sending a request-
correlated S1 reply. The probe demonstrates a local typed initialization
error; it does not establish a typed protocol error delivered to the caller.

Root performed fresh catalog discovery and Jev PRE before adding the tests.
The open gate selected `data_integrity.mutually_consistent`, verify mode,
subject `startup_reply`, with unchanged complete facts and no unknowns.
The desired correlated-error policy passed (`sol_5589f1afb6e042ab`);
a stderr-only control was rejected (`sol_a1978a2295844ce7`). The supplied
byte limits reuse existing caller fixtures and do not become product defaults.

Fresh actual-output Jev POST also opened with the unchanged allowed policy.
All three actual verification requests failed:

- Both limits: `sol_7857d919b4c6450a`.
- One-byte limit: `sol_edc162aac8a94d04`.
- Regular limit: `sol_42c377abda4b468a`.

All five receipts were persisted and reloaded. Exact Jev provenance, original
family requests, results and complete lowered reloads are retained in
`solve-review.full.json.gz`. Raw logs, source/build identity and checksums are
in this folder. Run `python3 verify.py`; its source checks use immutable Git
objects so future fixes do not rewrite this historical rejection.

The focused fix must send a valid correlated Error for a valid Hello when
native initialization fails, before terminating normally. Preserve version,
generation and request ID, and enforce the caller's message-byte bound. Do
not return Ready/Closed on failed initialization or silently report success.
Detailed diagnostics may remain on stderr. Preserve real cleanup, the
existing tiny-fixture probe and product Unavailable behavior until S4.

The prior loader risk is distinct: the worker narrowly maps the wrapper's
Rust panic during initial API loading, before environment creation, into a
typed local error. This cannot catch native aborts. After a failed loader,
do not retry native APIs in a possibly poisoned loader state. The existing
sandbox `ps` limitation was handled by the authorized client-suite rerun;
it does not require a production change.

No four numerical owner counts were measured, and root supplies no synthetic
zero-counter POST. Real checked Busy/success, reviewed ReleaseEnv ownership
and raw subprocess statuses support the bounded native cases; broad wrapper
safety and other platforms remain outside this observation. The protocol fix
must not restart the general O1 ownership audit.

Commands used `SPUR_REMOTE=0 TMPDIR=/private/tmp CARGO_NET_OFFLINE=true
CARGO_TARGET_DIR=/private/tmp/rust-gap-g1-acceptance-target` and the pinned
`SPUR_ORT_ARCHIVE` from the handoff, through `scripts/spur-cargo` only.
Baseline: `test -p spur-embedding-worker --features test-support -- --nocapture`
and `build -p spur-embedding-worker`. New RED: `test -p spur-embedding-worker
--features test-support --test native_lifecycle root_initialization_failure
-- --nocapture`. Inherited vendor warnings are byte-preserved. Local format
and diff checks passed.
