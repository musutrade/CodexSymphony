#!/usr/bin/env python3
"""Operator-owned bounded candidate registration; no installation or restart.

Only an independently reviewed audit is accepted. The installed entrypoint and
pin directory are outside agent mounts. No candidate-supplied module is imported.
"""
import argparse
import fcntl
import json
from pathlib import Path
import os

import evidence_admission as admission


def register(config, audit, head, expected_base):
    if config.get('mode') != 'verify-only' or config.get('candidate_registration') is not True:
        raise admission.Rejected('bounded candidate registration is not installed')
    approval = json.loads(Path(config['gate_approval']).read_text())
    admission.check_runtime(approval)
    admission.reviewed_audit(audit, approval)
    # The existing pin budget and TTL are authorities; registration adds no store.
    root = Path(config['pins'])
    descriptor = admission.pins.open_regular(root / 'registration.lock', os.O_RDWR)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with admission.ledger.hold(Path(config['publication_ledger'])):
            record = admission.ledger.load_record(config['publication_ledger'], audit['validation_id'])
            record = admission.ledger.verify(config['publication_ledger'], audit['validation_id'], record['inputs'])
            admission.check_record(record)
            admission.check_binding(audit, record, record['inputs']['tree'])
            token = admission.token_of(config)
            prefix = '/repos/' + config['repository']
            head = admission.object_id(head, 'candidate commit')
            tree, parents = admission.remote_commit(prefix, token, head)
            base = admission.main_head(prefix, token)
            if base != expected_base or tree != audit['tree'] or not admission.contains(prefix, token, base, head):
                raise admission.Rejected('candidate or main changed; reconcile explicitly before registration')
            admission.check_verification_inputs(config, approval, record, head, base, None, parents)
            identity = admission.pins.digest(admission.pins.canonical(audit))
            for pin_id, value in admission.pins.current(root).items():
                publication = value['publication']
                if publication.get('kind') != 'candidate-registration':
                    continue
                saved = publication['audit']
                if (saved['approval'], saved['tree']) != (audit['approval'], audit['tree']):
                    continue
                if publication.get('status') != 'confirmed' or saved != audit or \
                        admission.pins.digest(admission.pins.canonical(publication)) != value['publication_id']:
                    raise admission.Rejected('concurrent or conflicting registration; release its consumer explicitly')
                return {'status': 'REGISTERED', 'audit_sha256': identity, 'pin_id': pin_id,
                        'expires_at_ms': value['expires_at_ms'], 'reused_registration': True}
            bound = admission.subject(config['repository'], None, head, base, tree)
            value = admission.pins.pin(root, admission.pins.records_budget(config['storage_deployment']),
                        record, bound, 'candidate/' + identity, config['pin_ttl_seconds'],
                        {'kind': 'candidate-registration', 'audit': audit})
            try:
                with admission.pins.holding(root, value['pin_id'], record['validation_id']) as locked:
                    # A late result must not be confirmed after main changed.
                    if admission.main_head(prefix, token) != base:
                        raise admission.Rejected('main changed during registration; reconcile explicitly')
                    value = admission.pins.confirm(locked, value['pin_id'], record['validation_id'])
            except Exception:
                admission.pins.release(root, {value['pin_id']})
                raise
            return {'status': 'REGISTERED', 'audit_sha256': identity, 'pin_id': value['pin_id'],
                    'expires_at_ms': value['expires_at_ms'], 'reused_registration': False}
    finally:
        os.close(descriptor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--reviewed-audit', type=Path, required=True)
    parser.add_argument('--head', required=True)
    parser.add_argument('--expected-base', required=True)
    args = parser.parse_args()
    # The operator chooses the reviewed document; its contents are copied into
    # the capacity-charged pin, so a subsequent file replacement cannot change it.
    audit = admission.pins.read_json(args.reviewed_audit)
    print(json.dumps(register(admission.pins.read_json(args.config), audit, args.head, args.expected_base)))


if __name__ == '__main__':
    main()
