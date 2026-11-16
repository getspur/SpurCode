python3 -m venv .spur/venvs/graph-duckdb-2
export GRAPH_PYTHON="$PWD/.spur/venvs/graph-duckdb-2/bin/python"
"$GRAPH_PYTHON" -m pip install -r crates/spur-graph-duckdb/requirements-test.txt
export RELEASE_RUN="$PWD/.spur/releases/$("$GRAPH_PYTHON" -c 'import uuid; print(uuid.uuid4().hex)')"
mkdir -p "$(dirname "$RELEASE_RUN")"
PYTHONDONTWRITEBYTECODE=1 SPUR_SCCACHE_S3=0 SPUR_NO_LOCAL_FALLBACK=1 \
  "$GRAPH_PYTHON" crates/spur-graph-duckdb/scripts/check_release.py \
  --out "$RELEASE_RUN" --host-native --expect-platform osx_arm64
export SPUR_GRAPH_EXTENSION="$RELEASE_RUN/spur_graph.duckdb_extension"
"$GRAPH_PYTHON" crates/spur-graph-duckdb/scripts/verify_package.py "$SPUR_GRAPH_EXTENSION"
