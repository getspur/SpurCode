"""Run the unchanged ordinary API, native packaging and consumer regressions."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'crates/spur-graph-duckdb/tests'))
names = ['test_extension', 'test_temporal', 'test_file_facts', 'test_diagnostics',
         'test_contract', 'test_package', 'test_app_consumer']
suite = unittest.defaultTestLoader.loadTestsFromNames(names)
result = unittest.TextTestRunner(verbosity=2).run(suite)
assert not result.skipped, result.skipped
assert not result.expectedFailures, result.expectedFailures
sys.exit(0 if result.wasSuccessful() else 1)
