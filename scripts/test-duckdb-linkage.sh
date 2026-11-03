#!/usr/bin/env bash
# Cargo's resolved features, including dev dependencies, are the contract.
set -euo pipefail
cd "$(dirname "$0")/.."
for package in spur-analyst spur-core spur-cli spur-code-eval; do
    features=$(scripts/spur-cargo tree --locked --offline -p "$package" \
        --no-default-features -e features -i libduckdb-sys)
    if [[ "$features" == *'libduckdb-sys feature "bundled"'* ]]; then
        echo "FAIL: $package --no-default-features still compiles DuckDB" >&2
        exit 1
    fi
    features=$(scripts/spur-cargo tree --locked --offline -p "$package" \
        -e features -i libduckdb-sys)
    if [[ "$features" != *'libduckdb-sys feature "bundled"'* ]]; then
        echo "FAIL: $package default build lost bundled DuckDB" >&2
        exit 1
    fi
    echo "PASS: $package supports prebuilt and bundled linkage"
done

# The full CLI UI/analytics feature set must also work without source linking.
features=$(scripts/spur-cargo tree --locked --offline -p spur-cli \
    --no-default-features --features embed,tui-default,interactive-default,duckdb \
    -e features -i libduckdb-sys)
if [[ "$features" == *'libduckdb-sys feature "bundled"'* ]]; then
    echo "FAIL: CLI runtime features re-enable bundled DuckDB" >&2
    exit 1
fi
echo "PASS: full CLI runtime features support prebuilt linkage"
