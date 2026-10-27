# Run after stackcollapse-sample.awk. Preserve waits for honest wall-stack accounting.
BEGIN { for (symbol in _IGNORE) delete _IGNORE[symbol] }
