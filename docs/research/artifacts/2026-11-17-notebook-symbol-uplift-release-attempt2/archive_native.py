"""Archive only this attempt's standalone output; never rewrite old evidence."""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = Path('/private/tmp/spur-notebook-u5-bd-c1v3-attempt2-native')


def main():
    target = HERE / 'native'
    target.mkdir()
    assert json.loads((SOURCE / 'evidence.json').read_bytes())['status'] == 'passed'
    files = {}
    for path in sorted(SOURCE.iterdir()):
        if path.is_file():
            data = path.read_bytes()
            with (target / path.name).open('xb') as stream:
                stream.write(data)
            files[path.name] = hashlib.sha256(data).hexdigest()
    with (target / 'archive.json').open('x') as stream:
        stream.write(json.dumps(dict(original=str(SOURCE), files=files), indent=2) + '\n')
    print(json.dumps(dict(status='passed', original=str(SOURCE), archived_files=len(files))))


if __name__ == '__main__':
    main()
