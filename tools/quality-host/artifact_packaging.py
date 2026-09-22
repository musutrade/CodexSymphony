"""Losslessly pack source-bound coverage without sharing artifact identities."""
import gzip
import hashlib
import lzma
from pathlib import Path


def compact_artifacts(response, root):
    root = Path(root)
    if not root.is_absolute() or root.resolve() != root:
        raise ValueError('noncanonical artifact root')
    refs = response.get('artifacts', [])
    nested = [ref for evidence in response.get('collection', {}).get('evidence', [])
              for ref in evidence.get('artifacts', [])]
    by_id, paths = {}, {}
    for ref in refs:
        if ref['id'] in by_id and by_id[ref['id']] != ref:
            raise ValueError('conflicting artifact identity')
        by_id[ref['id']] = ref
        relative = Path(ref['path'])
        if relative.is_absolute() or '..' in relative.parts or str(relative) != ref['path']:
            raise ValueError('unsafe artifact path')
        path = root / relative
        if path.resolve() != path or not path.is_file():
            raise ValueError('unsafe artifact file')
        data = path.read_bytes()
        if len(data) != ref['bytes'] or hashlib.sha256(data).hexdigest() != ref['sha256']:
            raise ValueError('artifact bytes differ from evidence')
        paths[ref['path']] = data
    for ref in nested:
        if by_id.get(ref['id']) != ref:
            raise ValueError('nested artifact differs from envelope')
    # Core requires a distinct path/descriptor for each source. Keep those
    # identities; compress complete evidence bytes, never truncate/filter counters.
    packed, updates = {}, {}
    for ref in refs:
        name = ref['path']
        is_gzip = ref.get('media_type') == 'application/gzip' and name.endswith('.gz')
        is_json = ref.get('media_type') == 'application/json' and name.endswith('.json')
        if not (is_gzip or is_json):
            continue
        data = paths[name]
        raw = gzip.decompress(data) if is_gzip else data
        digest = hashlib.sha256(raw).hexdigest()
        if digest not in packed:
            encoded = lzma.compress(raw, filters=[{'id': lzma.FILTER_LZMA2,
                                                     'preset': 9 | lzma.PRESET_EXTREME,
                                                     'lc': 4, 'pb': 0}])
            if lzma.decompress(encoded) != raw:
                raise ValueError('lossless coverage encoding verification failed')
            packed[digest] = encoded
        encoded = packed[digest]
        if len(encoded) >= len(data):
            continue
        target = (name[:-3] if is_gzip else name) + '.xz'
        if (root / target).exists() or (root / target).is_symlink():
            raise ValueError('coverage output path already exists')
        updates[name] = (target, encoded)
    # Validation and encoding complete before mutating the generated outputs.
    for name, (target, data) in updates.items():
        with (root / target).open('xb') as stream:
            stream.write(data)
    for ref in refs + nested:
        if ref['path'] in updates:
            target, data = updates[ref['path']]
            ref.update(path=target, sha256=hashlib.sha256(data).hexdigest(),
                       bytes=len(data), media_type='application/x-xz')
    for name in updates:
        (root / name).unlink()
    return response
