export SPUR_RELEASE_SOURCE_IDENTITY="$("$GRAPH_PYTHON" -c \
  'import json,os; print(json.load(open(os.path.join(os.environ["RELEASE_RUN"],"receipt.json")))["release_trust"]["source_identity"])')"
"$GRAPH_PYTHON" crates/spur-graph-duckdb/examples/app_consumer.py \
  --fixture crates/spur-graph-duckdb/tests/fixtures/app_consumer.json \
  --output "$RELEASE_RUN/headless-demo" --package "$SPUR_GRAPH_EXTENSION" \
  --extractor-id "$SPUR_RELEASE_SOURCE_IDENTITY"
