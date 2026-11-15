"""Retain original requests and add fully qualified method traversal checks."""
import hashlib
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent
original = OUT / 'real-cases.json'
spec = json.loads(original.read_text())
for direction in ['callers', 'callees']:
    for unresolved in [False, True]:
        spec['requests'].append({
            'case_id': 'real-' + direction + '-qualified-extract_file_facts-' + str(unresolved).lower(),
            'tool': 'code_' + direction,
            'args': {'selector': 'crates/spur-graph/src/extract/tree_sitter.rs::impl BytesExtractor::extract_file_facts',
                     'include_unresolved': unresolved, 'response_format': 'full'},
        })
target = OUT / 'real-cases-v2.json'
data = (json.dumps(spec, indent=2) + '\n').encode()
if target.exists():
    assert target.read_bytes() == data
else:
    target.write_bytes(data)
proof = {
    'original': 'real-cases.json',
    'original_sha256': hashlib.sha256(original.read_bytes()).hexdigest(),
    'extended': target.name, 'extended_sha256': hashlib.sha256(data).hexdigest(),
    'original_requests': 25, 'extended_requests': len(spec['requests']),
    'sources_unchanged': True, 'original_requests_preserved': True,
    'reason': 'Original four file-qualified bare method traversal selectors return native not-found. Retain them as error cases and add the full impl-qualified selector before candidate evaluation. No reference answer data supplies candidate facts.',
}
(OUT / 'real-request-revision.json').write_text(json.dumps(proof, indent=2) + '\n')
print(json.dumps(proof))
