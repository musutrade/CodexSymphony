"""Bound completed Gate records; retain current proof and explicit expiry facts.

Retention pins (evidence_pins) are honored here: the caller holds the exclusive pin
lock across this whole pass and a pinned run is never retired. Record bytes are
evidence_pins.charge, the measure pin admission uses: the live pins, the current
record and the pin state are charged first and must fit, or the pass fails; an older
unpinned record is retained only if it still fits.
"""
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import time

import evidence_pins
import fixed_workspace
import manual_measure
import rust_capture


def safe_directory(path):
    path = fixed_workspace.canonical(path)
    if path.is_mount() or not shutil.rmtree.avoids_symlink_attacks:
        raise ValueError('unsafe record cleanup root')
    device = path.stat().st_dev
    for directory, dirs, files in os.walk(path, followlinks=False):
        for entry in [Path(directory), *(Path(directory) / name for name in dirs)]:
            if entry.is_mount() or entry.lstat().st_dev != device:
                raise ValueError('record contains a foreign mount')
    return path


def deduplicate(run, apply):
    reports = run / 'reports'
    if not reports.exists():
        return 0
    safe_directory(reports)
    seen, released = {}, 0
    for path in sorted(reports.rglob('*')):
        if path.is_symlink():
            raise ValueError('aliased report')
        if path.is_file():
            released += deduplicate_file(path, seen, apply)
    return released


def deduplicate_file(path, seen, apply):
    stat = path.stat()
    identity = (stat.st_size, stat.st_mode & 0o777, rust_capture.digest(path))
    prior = seen.setdefault(identity, path)
    if prior.stat().st_ino == stat.st_ino:
        return 0
    if apply:
        temporary = path.with_name(path.name + '.deduplicate-new')
        if temporary.is_symlink():
            raise ValueError('unfinished report deduplication')
        if temporary.exists():
            if temporary.stat().st_ino != prior.stat().st_ino:
                raise ValueError('unfinished report deduplication')
        else:
            os.link(prior, temporary)
        temporary.replace(path)
    return stat.st_size


def summary(run):
    archive = json.loads((run / 'source-archive.json').read_text())
    record = {'run': run.name, 'expired_at': time.time(), 'artifacts_available': False,
              'source_archive_sha256': archive['sha256'],
              'source_inputs_sha256': rust_capture.digest(run / 'source-inputs.json')}
    for name in ('complete-gate.json', 'measurement-summary.json', 'recovery-measurement.json', 'capture-registration.json'):
        path = run / name
        if path.exists():
            record[name] = json.loads(path.read_text())
    timing = run / 'timings.jsonl'
    record['timings'] = [json.loads(line) for line in timing.read_text().splitlines()] if timing.exists() else []
    report = run / 'reports/test_result.json'
    if report.exists():
        value = json.loads(report.read_text())
        record['report'] = {key: value[key] for key in ('passed', 'evidence_complete', 'configuration_digest', 'source_identity')}
        record['report']['sha256'] = rust_capture.digest(report)
    return record


def read_index(parent):
    path = fixed_workspace.canonical(parent / 'expired-records.json')
    values = json.loads(path.read_text()) if path.exists() else []
    for value in values:
        if not re.fullmatch(r'run-[0-9a-f]{12}', value['run']) or value['artifacts_available'] is not False:
            raise ValueError('invalid expiry index')
    return values


def resume_deletions(parent, records, pinned, apply):
    indexed = {row['run'] for row in records}
    for path in parent.glob('retiring-run-*'):
        name = path.name.removeprefix('retiring-')
        if name not in indexed:
            raise ValueError('unregistered record retirement')
        if parent / name in pinned:
            raise ValueError('pinned record is being retired')
        safe_directory(path)
        if apply:
            shutil.rmtree(path)


def retire(run, parent, records, pinned, apply):
    if run in pinned:
        raise ValueError('pinned record cannot be retired')
    record = summary(run)
    safe_directory(run)
    if apply:
        target = parent / ('retiring-' + run.name)
        if target.exists() or target.is_symlink():
            raise ValueError('record retirement collision')
        records[:] = [row for row in records if row['run'] != run.name] + [record]
        fixed_workspace.write_json(parent / 'expired-records.json', records)
        run.rename(target)
        shutil.rmtree(target)
    return {'run': str(run), 'action': 'retire-completed-record', 'artifacts_available': not apply}


def trim_index(parent, records, hours, budget, apply):
    cutoff = time.time() - hours * 3600
    kept = []
    for row in sorted(records, key=lambda row: row['expired_at'], reverse=True):
        size = len(json.dumps([*kept, row], sort_keys=True).encode())
        if row['expired_at'] >= cutoff and size <= budget:
            kept.append(row)
    if apply:
        path = parent / 'expired-records.json'
        if budget < 2:
            path.unlink(missing_ok=True)
        else:
            fixed_workspace.write_json(path, kept)


def kept(index, age, fits, keep, hours):
    """Retained within count and age; an older record also within what pins and newer records leave."""
    return index < keep and age < hours * 3600 and (index == 0 or fits)


def maintain(parent, candidates, keep, hours, budget, apply, pins):
    """pins: the live pin state, read under the pin lock the caller holds for this pass.

    The pins, the current record (candidates[0]) and the pin state are charged first,
    exactly as pin admission charges them; if they do not fit the pass fails before
    any change. The current record still expires by count and age."""
    records = read_index(parent)
    pinned = evidence_pins.runs(pins)
    candidates.sort(key=evidence_pins.archived_at, reverse=True)
    needed = evidence_pins.reserved(pins, candidates[:1])
    if needed > budget:
        raise ValueError(f'pins and current required evidence need {needed} of {budget} record bytes')
    resume_deletions(parent, records, pinned, apply)
    retained, results = list(pinned), []
    for index, run in enumerate(candidates):
        released = deduplicate(run, apply)
        age = time.time() - evidence_pins.archived_at(run)
        result = {'run': str(run), 'deduplicated_bytes': released, 'retained': True}
        if run in pinned:
            results.append(result | {'pinned': True})
        elif kept(index, age, charged(pins, [*retained, run]) <= budget, keep, hours):
            retained.append(run)
            results.append(result)
        else:
            results.append(retire(run, parent, records, pinned, apply))
    unregister_expired(parent, records, apply)
    trim_index(parent, records, hours, max(0, budget - charged(pins, retained)), apply)
    return results


def charged(pins, runs):
    return evidence_pins.charge(runs) + evidence_pins.state_bytes(pins)


def unregister_expired(parent, records, apply):
    if not apply:
        return
    state = manual_measure.DEPLOYMENT.parent
    path = fixed_workspace.canonical(state / 'capture-caches.json')
    with (state / 'maintenance.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        value = json.loads(path.read_text())
        if value['schema'] != 'capture-cache-registry/v1':
            raise ValueError('invalid capture registry')
        expired = {str(parent / row['run']) for row in records if not (parent / row['run']).exists()}
        value['captures'] = [row for row in value['captures'] if row['root'] not in expired]
        fixed_workspace.write_json(path, value)
