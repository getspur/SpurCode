# Named Deno configuration recheck

**Historical runtime observation:** the subsequent [new-build verification](../2026-11-18-ts-named-new-build/README.md) confirms that named TS/Deno discovery, saving and cell selection are now exposed. It records a separate MCP startup blocker. The older interface responses below are retained as evidence of the earlier build.

The user's correction is valid: the current source checkout declares named Deno configurations, including a `project_root`, stable profile identity, defaults and per-cell selectors. The earlier diagnostic notebook tested the standalone Deno kernel, not this named runtime path.

[Updated diagnostic notebook](/Users/kevintruong/project/spur-notebook/2026-11-18-deno-project-configuration.ipynb) · [Captured correction](experimental-notebook.ipynb) · [Results](results.json) · [Live schemas](live-tool-schemas.json) · [Read-only MCP responses](mcp-receipts.json)

## Live result

The original notebook MCP connection closed during the app restart. The recheck used the production length-prefixed MCP protocol and queried current app endpoints directly. The connected writer-owned endpoint still:

- Advertises C++/Rust kernels only in `notebook_kernel_config_spec` and `notebook_configure_kernel`.
- Rejects `kernel_name: "deno"` with error `-32602`, `unsupported logical kernel`.
- Returns profile registry schema version 2, with C++ and Rust families and no Deno family.

The live source file `/Users/kevintruong/project/spur-notebook/src/mcp/tools/notebook_kernel_config_spec.rs` advertises Deno and describes the new named runtime contract. Its source hash and the installed app's binary hash are recorded. The observed installed app lacks the expected `Contents/Resources/deno-host-runtime` directory. These observations establish a source/interface mismatch; they do not identify every cause of the build or packaging difference.

The desktop app and recently recreated workspace endpoints were checked. A notebook update initially hit another process's writer lease and was rejected; the update was then sent through the actual lease owner. No lock was removed or bypassed. The correction is now saved through MCP near the beginning of the notebook. All seven original executed code cells and their outputs are retained.

**Named-profile import execution remains unverified.** No named profile was created or changed, no default/selector was altered, and no app was replaced or rebuilt. A question about which updated build to use remains pending with the user.

## Next probe

1. Connect to a compatible build whose live schema includes `deno`.
2. Discover the actual configuration schema and existing profile names.
3. Preview a new task-owned profile pointing to the local-only alias/JSON/JSX fixture.
4. Save it through the version/UUID guarded writer, select it on a dedicated probe cell, then run the alias import.
5. Record actual profile/runtime identity and import results. Keep the standalone results as a separate comparison.

`mcp_client.py` is a small production-protocol client used to recover the closed connection. It supports `SPUR_RECHECK_SOCKET` for an explicitly selected local endpoint. Notebook changes were made using `notebook_insert_cell`; the captured `.ipynb` was copied only after an MCP document read. No raw notebook authoring was used.
