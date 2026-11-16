# External catalog package search implementation

1. Add behavior tests through SQL `list_packages` and the Parquet MCP backend.
   Cover mixed case, multiple keywords and separators, one edit/transposition,
   short/unrelated terms, empty/punctuation filters, Unicode, source isolation,
   cursor totals/order, and exact package browsing. Run remotely through
   `scripts/spur-cargo`, observe intended RED, and commit the tests.
2. Add a shared private name matcher with the solved one-edit/three-character
   policy, wire both package-listing paths, and update tool/schema documentation.
   Preserve symbol filters and coordinate descent. Run targeted GREEN and the
   crate suite; format only affected Rust files. Re-solve shipped policy and
   commit the implementation.
3. Brain reviews and deploys the committed Code Lambda artifact with rollback
   information and live smoke checks. No Terraform or worker changes are needed.
