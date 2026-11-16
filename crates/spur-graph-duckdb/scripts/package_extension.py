"""Stage, probe and publish a native library for DuckDB exactly 2.0.0.

Usage is unchanged: --library RAW_NATIVE_LIBRARY --out spur_graph.duckdb_extension.
The output and its .json sidecar are relative symlinks through one atomic
current-directory pointer into .<output-name>.packages. Complete generations
are retained; no prior artifact is overwritten or garbage-collected. Copy both
dereferenced files for a portable package. Unmanaged existing outputs are
refused: choose a fresh --out rather than overwrite a historical release.

Publication requires local POSIX symlinks, flock and same-filesystem atomic
rename. A failure before the pointer swap leaves the previous release readable;
an interrupted attempt can leave unreferenced staging/complete generations.
Atomic visibility is not a sudden-power-loss durability guarantee. The sidecar
records unsigned/local artifact identity, never a signature or saved readiness.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import tempfile
import uuid

try:
    import fcntl
except ImportError:
    fcntl = None

import duckdb

from verify_package import (ABI, API_VERSION, TRUST, checksum, extension_footer,
                            probe_artifact, runtime_contract, verify_package)


def package_extension(library, out):
    if fcntl is None:
        raise ValueError('package publication requires POSIX flock and symlinks')
    library = Path(library).resolve(strict=True)
    out = Path(os.path.abspath(out))  # Keep the public alias, not its old target.
    if library == out.resolve():
        raise ValueError('the output must differ from the raw library')
    if duckdb.__version__ != API_VERSION:
        raise ValueError('packaging runtime must be DuckDB exactly ' + API_VERSION)
    out.parent.mkdir(parents=True, exist_ok=True)
    store = out.parent / ('.' + out.name + '.packages')
    if store.is_symlink():
        raise ValueError('package generation directory must not be a symlink')
    store.mkdir(exist_ok=True)
    aliases = {out: Path(store.name) / 'current' / out.name,
               Path(str(out) + '.json'): Path(store.name) / 'current' / (out.name + '.json')}
    # Persistent inode; removing it would break serialization between publishers.
    with (store / '.publish.lock').open('a+b') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for alias, target in aliases.items():
            if os.path.lexists(alias) and (not alias.is_symlink() or os.readlink(alias) != str(target)):
                raise ValueError(f'refusing to overwrite unmanaged package output: {alias}')
        current = store / 'current'
        if os.path.lexists(current) and (not current.is_symlink()
                or Path(os.readlink(current)).name != os.readlink(current)
                or not os.readlink(current).startswith('release-')):
            raise ValueError('refusing to replace unmanaged package current pointer')
        staging = Path(tempfile.mkdtemp(prefix='.staging-', dir=store))
        pointer = store / ('.current-' + uuid.uuid4().hex)
        created_aliases = []
        published = False
        try:
            with duckdb.connect() as connection:
                version, platform = runtime_contract(connection)
            if version != 'v' + API_VERSION:
                raise ValueError('packaging requires native runtime exactly v' + API_VERSION)
            # Bootstrap with an empty footer extension version. The authoritative
            # version and source identity come from this library's native info.
            bootstrap = staging / 'probe' / out.name
            bootstrap.parent.mkdir()
            shutil.copyfile(library, bootstrap)
            with bootstrap.open('ab') as stream:
                stream.write(extension_footer('', platform))
            info = probe_artifact(bootstrap, bootstrap=True)
            binary = staging / out.name
            shutil.copyfile(library, binary)
            with binary.open('ab') as stream:
                stream.write(extension_footer(info['extension_version'], platform))
                stream.flush()
                os.fsync(stream.fileno())
            shutil.rmtree(bootstrap.parent)
            manifest = {
                'format_version': 1, **info, 'abi': ABI, 'platform': platform,
                'source_identity': info['extractor_id'], 'trust': dict(TRUST),
                'binary': {'name': out.name, 'sha256': checksum(binary),
                           'size_bytes': binary.stat().st_size},
            }
            with Path(str(binary) + '.json').open('x') as stream:
                json.dump(manifest, stream, indent=2, sort_keys=True)
                stream.write('\n')
                stream.flush()
                os.fsync(stream.fileno())
            # Fresh final load checks the actual bytes/sidecar to be published.
            verify_package(binary)
            final = store / ('release-' + uuid.uuid4().hex)
            staging.rename(final)
            for alias, target in aliases.items():
                if not os.path.lexists(alias):
                    alias.symlink_to(target)
                    created_aliases.append(alias)
            pointer.symlink_to(final.name, target_is_directory=True)
            # Commit point for BOTH stable aliases. No fallible work after it.
            os.replace(pointer, current)
            published = True
            return manifest
        except BaseException:
            if not published:
                for alias in created_aliases:
                    alias.unlink()
            raise
        finally:
            if not published:
                pointer.unlink(missing_ok=True)
                if staging.exists():
                    shutil.rmtree(staging)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest = package_extension(args.library, args.out)
    except (ValueError, OSError, duckdb.Error) as error:
        parser.exit(1, f'Package failed: {error}\n')
    print(f"Packaged {args.out}: DuckDB v{API_VERSION}, {manifest['platform']}, {ABI}; "
          f"unsigned local SHA256 {manifest['binary']['sha256']}; sidecar {args.out}.json")


if __name__ == '__main__':
    main()
