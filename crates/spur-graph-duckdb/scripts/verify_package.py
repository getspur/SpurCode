"""Verify a local unsigned spur_graph package with DuckDB exactly 2.0.0.

The version 1 JSON sidecar is artifact data, not a saved readiness result or a
signature. SHA256 detects byte changes against that local receipt; it does not
authenticate a publisher. Every successful verify_package call also loads the
checksummed artifact on a fresh native connection. The CLI reports that live
observation separately. No INSTALL, network access, or runtime upgrade occurs.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re

import duckdb


API_VERSION = '2.0.0'
ABI = 'C_STRUCT_UNSTABLE'
TRUST = {'mode': 'unsigned_local', 'signature': 'absent'}
# DuckDB extension-ci-tools 2e5d22fa1b1c42e6fc7aa84a8e66a15e6b492b30,
# scripts/append_extension_metadata.py: eight 32-byte fields, 256-byte signature.
FOOTER_PREFIX = b'\x00\x93\x04\x10duckdb_signature\x80\x04'
FIELD_SIZE = 32
SIGNATURE_SIZE = 256
FOOTER_SIZE = len(FOOTER_PREFIX) + 8 * FIELD_SIZE + SIGNATURE_SIZE
INFO_FIELDS = {'extension_version', 'facts_schema_version', 'duckdb_api_version',
               'extractor_id', 'supported_languages', 'resolution_scope'}
MANIFEST_FIELDS = INFO_FIELDS | {'format_version', 'binary', 'abi', 'platform',
                                 'source_identity', 'trust'}


def checksum(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def extension_footer(version, platform):
    fields = ['', '', '', ABI, version, 'v' + API_VERSION, platform, '4']
    try:
        encoded = [field.encode('ascii') for field in fields]
    except (AttributeError, UnicodeError) as error:
        raise ValueError('extension footer fields must be ASCII strings') from error
    if any(len(field) > FIELD_SIZE or b'\0' in field for field in encoded):
        raise ValueError('extension footer field exceeds its upstream 32-byte slot or contains NUL')
    return (FOOTER_PREFIX + b''.join(field.ljust(FIELD_SIZE, b'\0') for field in encoded)
            + bytes(SIGNATURE_SIZE))


def read_footer(path):
    with Path(path).open('rb') as stream:
        if stream.seek(0, 2) < FOOTER_SIZE:
            raise ValueError('artifact has no complete DuckDB extension footer')
        stream.seek(-FOOTER_SIZE, 2)
        footer = stream.read()
    if not footer.startswith(FOOTER_PREFIX):
        raise ValueError('invalid DuckDB extension footer prefix')
    if footer[-SIGNATURE_SIZE:] != bytes(SIGNATURE_SIZE):
        raise ValueError('unsigned local package must have an absent (zero) signature')
    fields = []
    for offset in range(len(FOOTER_PREFIX), FOOTER_SIZE - SIGNATURE_SIZE, FIELD_SIZE):
        raw = footer[offset:offset + FIELD_SIZE]
        try:
            field = raw.rstrip(b'\0').decode('ascii')
        except UnicodeError as error:
            raise ValueError('invalid ASCII extension footer field') from error
        if b'\0' in raw.rstrip(b'\0'):
            raise ValueError('noncanonical extension footer padding')
        fields.append(field)
    if fields[:3] != ['', '', ''] or fields[7] != '4':
        raise ValueError('unsupported extension footer format')
    return dict(zip(('abi', 'extension_version', 'runtime_version', 'platform'), fields[3:7]))


def validate_info(info):
    if not isinstance(info, dict) or set(info) != INFO_FIELDS:
        raise ValueError('spur_graph_info metadata fields do not match the supported contract')
    if info['duckdb_api_version'] != API_VERSION:
        raise ValueError('artifact requires DuckDB API version exactly ' + API_VERSION)
    if type(info['facts_schema_version']) is not int or info['facts_schema_version'] != 1:
        raise ValueError('unsupported facts schema version in artifact metadata')
    if not isinstance(info['extension_version'], str) or not info['extension_version']:
        raise ValueError('missing extension version in artifact metadata')
    if not isinstance(info['extractor_id'], str) or not re.fullmatch(
            r'source-sha256-v1:[0-9a-f]{64}', info['extractor_id']):
        raise ValueError('invalid extractor source identity in artifact metadata')
    languages = info['supported_languages']
    if (not isinstance(languages, list) or not languages
            or any(not isinstance(language, str) or not language for language in languages)
            or languages != sorted(set(languages))):
        raise ValueError('invalid supported languages in artifact metadata')
    if info['resolution_scope'] != 'single_file':
        raise ValueError('unsupported resolution scope in artifact metadata')


def runtime_contract(connection):
    return (connection.execute('SELECT version()').fetchone()[0],
            connection.execute('PRAGMA platform').fetchone()[0])


def unique_json_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate manifest field: ' + key)
        result[key] = value
    return result


def probe_artifact(binary, *, bootstrap=False):
    """Load only a staged/verified binary, never a source-tree-derived identity."""
    if duckdb.__version__ != API_VERSION:
        raise ValueError('packaging/verification runtime must be DuckDB exactly ' + API_VERSION)
    footer = read_footer(binary)
    if footer['abi'] != ABI:
        raise ValueError(f"incompatible ABI: {footer['abi']!r}; required {ABI}")
    if footer['runtime_version'] != 'v' + API_VERSION:
        raise ValueError('incompatible footer runtime version: ' + footer['runtime_version'])
    with duckdb.connect(config={'allow_unsigned_extensions': True}) as connection:
        version, platform = runtime_contract(connection)
        if version != 'v' + API_VERSION:
            raise ValueError(f'incompatible native runtime {version}; required v{API_VERSION}')
        if platform != footer['platform']:
            raise ValueError(f"incompatible platform: artifact {footer['platform']}, runtime {platform}")
        try:
            connection.execute("LOAD '" + str(Path(binary).resolve()).replace("'", "''") + "'")
            info = connection.execute('SELECT spur_graph_info()').fetchone()[0]
        except duckdb.Error as error:
            raise ValueError(f'native artifact load/info probe failed: {error}') from error
        validate_info(info)
        if not bootstrap and info['extension_version'] != footer['extension_version']:
            raise ValueError('loaded info extension version disagrees with artifact footer')
        return info


def verify_package(package):
    """Return artifact metadata only after integrity AND a fresh native load pass.

    Resolve the binary once so a publication cannot mix generations. The stable
    sidecar alias must identify the same generation. A concurrent pointer swap
    can fail this check; retry against the new generation or the resolved path.
    Portable copies must include both the binary and its adjacent .json file.
    """
    package = Path(package).absolute()
    try:
        binary = package.resolve(strict=True)
        if not binary.is_file():
            raise ValueError('package binary is not a file')
    except OSError as error:
        raise ValueError(f'missing package binary/artifact: {package}') from error
    try:
        sidecar = Path(str(package) + '.json').resolve(strict=True)
        if sidecar != Path(str(binary) + '.json'):
            raise ValueError('sidecar manifest and binary identify inconsistent package generations')
        manifest = json.loads(sidecar.read_text(), object_pairs_hook=unique_json_fields)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f'missing or invalid package sidecar manifest: {package}.json') from error
    if (not isinstance(manifest, dict) or set(manifest) != MANIFEST_FIELDS
            or type(manifest['format_version']) is not int or manifest['format_version'] != 1):
        raise ValueError('unsupported sidecar manifest format or fields')
    record = manifest['binary']
    if not isinstance(record, dict) or set(record) != {'name', 'sha256', 'size_bytes'}:
        raise ValueError('invalid manifest binary record')
    if record['name'] != binary.name:
        raise ValueError('manifest binary name does not match artifact')
    if type(record['size_bytes']) is not int or record['size_bytes'] != binary.stat().st_size:
        raise ValueError('artifact size mismatch')
    # Deliberately before runtime checks and native code loading.
    if record['sha256'] != checksum(binary):
        raise ValueError('artifact SHA256 checksum mismatch')
    if manifest['trust'] != TRUST:
        raise ValueError('unsupported trust mode; only unsigned local packages are supported')
    validate_info({key: manifest[key] for key in INFO_FIELDS})
    if manifest['source_identity'] != manifest['extractor_id']:
        raise ValueError('manifest source identity disagrees with extractor identity')
    footer = read_footer(binary)
    for key in ('abi', 'platform', 'extension_version'):
        if manifest[key] != footer[key]:
            raise ValueError(f'manifest {key} disagrees with artifact footer')
    info = probe_artifact(binary)
    if info != {key: manifest[key] for key in INFO_FIELDS}:
        raise ValueError('manifest metadata/identity disagrees with loaded spur_graph_info')
    # Catch changes during the probe as well as before it. Unsigned/local trust
    # assumes no adversarial concurrent filesystem writer; this is no signature.
    if record['sha256'] != checksum(binary):
        raise ValueError('artifact SHA256 checksum changed during native probe')
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    args = parser.parse_args()
    try:
        manifest = verify_package(args.package)
    except (ValueError, OSError, duckdb.Error) as error:
        parser.exit(1, f'Package verification failed: {error}\n')
    print(json.dumps({'artifact': manifest, 'live_readiness': {
        'status': 'verified_now', 'duckdb_version': API_VERSION,
        'platform': manifest['platform'], 'binary_sha256': manifest['binary']['sha256'],
    }}, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
