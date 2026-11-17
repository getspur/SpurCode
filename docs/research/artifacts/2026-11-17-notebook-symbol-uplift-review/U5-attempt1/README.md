# U5 attempt 1 root review

The release receipt verifier passes: REMOTE 61 core and 4 adapter tests, HOST 223 native Python tests with zero skips, all 325 native source inputs, 281 preserved evidence files, and the exact fresh package c5f36a0f. Real notebook acceptance retains 46/24 symbols, 17/11 functions, 28 anchors, 70 refs, 50 symbol changes and 10 cell events. The remote docs-fixture transport signal was fixed within the authorized driver/test paths and is resolved.

Request a documentation-only correction at release-contract.md:383-384. It claims the reader verifies table schemas/counts; approved read_result/validate_tables actually checks manifest/run/source identity and exported file paths, sizes and checksums. Schemas/counts are recorded during native publication. Independent reviewer bd-3mnu confirmed the mismatch with a no-write in-memory manifest overlay. No other Important/Critical issue was found in the release-driver or exact API schemas.

Correct the wording and produce new documentation-bound acceptance/hash evidence while preserving all prior immutable receipts and the verified native artifact. There is no requested native or consumer behavior change and no reason to repeat unchanged broad suites solely for prose.

Root notebook-runtime acceptance is in progress in a new notebook created through MCP. Its initial runtime was v1.5.5; the v2 candidate has not been loaded into that incompatible runtime. Actual DuckDB 2 execution and final integration remain pending. See review.json for the exact verified release identity and review scope.
