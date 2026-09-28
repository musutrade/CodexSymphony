"""Close a measured capture with a compact source archive, never a second checkout."""
import json
from pathlib import Path
import tarfile

import fixed_workspace
import rust_capture
from timing import phase


def archive_sources(run, root, inputs):
    archive = run / 'source.tar.gz'
    record = run / 'source-archive.json'
    if record.exists():
        saved = json.loads(record.read_text())
        if saved['inputs'] != inputs or rust_capture.digest(archive) != saved['sha256']:
            raise ValueError('source archive changed')
        return
    with tarfile.open(archive, 'x:gz') as output:
        for name in sorted(inputs):
            output.add(root / name, arcname=name, recursive=False)
    if fixed_workspace.sources(root) != inputs:
        raise ValueError('source changed while archiving')
    fixed_workspace.write_json(record, {'inputs': inputs, 'sha256': rust_capture.digest(archive),
                                       'coverage_root': str(root), 'scope': 'Git source inputs only'})


def complete(run, root, pending, result):
    if result['coverage_and_crap'] != 'PASS':
        raise ValueError('failed measurement cannot release capture')
    if rust_capture.digest(Path(result['measurement'])) != result['measurement_sha256']:
        raise ValueError('measurement changed before handoff')
    capture = json.loads(pending.read_text())
    if capture['capture'] != str(run):
        raise ValueError('pending capture identity changed')
    inputs = fixed_workspace.sources(root)
    if {name: item['sha256'] for name, item in inputs.items()} != capture['source_inputs']:
        raise ValueError('capture source changed before handoff')
    registration = json.loads((run / 'capture-registration.json').read_text())
    if registration['root'] != str(run) or registration['bundle_sha256'] != rust_capture.digest(run / 'probes/backend/bundle.json'):
        raise ValueError('registered capture identity changed')
    with phase(run, 'capture-source-handoff'):
        archive_sources(run, root, inputs)
        pending.unlink()
