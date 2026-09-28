import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).parents[1]))
import database_pool as db


class DatabasePoolTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.policy = {'postgres': {'image': 'fixture-image', 'image_id': 'fixture-id',
                                   'test': {'memory': 1024, 'memory_swap': 2048, 'nano_cpus': 1000000000}}}
        self.name = 'codexsymphony-gate-fixed-primary'
        self.info = {'Name': '/' + self.name, 'Image': 'fixture-id',
                     'Config': {'Labels': {'codexsymphony.owner': db.OWNER}},
                     'HostConfig': {'Memory': 1024, 'MemorySwap': 2048, 'NanoCpus': 1000000000,
                                    'LogConfig': {'Type': 'local', 'Config': {'max-size': '1m', 'max-file': '2'}},
                                    'Tmpfs': {'/var/lib/postgresql/data': 'rw,size=1024'}},
                     'State': {'Running': True}, 'Mounts': [],
                     'NetworkSettings': {'Ports': {'5432/tcp': [{'HostIp': '127.0.0.1', 'HostPort': '54321'}]}}}
        for name, value in (('VOLUME', self.base), ('DIRECTORY', self.base / 'db'), ('LEASES', {})):
            patcher = patch.object(db, name, value)
            patcher.start(); self.addCleanup(patcher.stop)
        patcher = patch.object(Path, 'is_mount', lambda path: path == self.base)
        patcher.start(); self.addCleanup(patcher.stop)

    def test_fixed_slot_reuses_container_and_resets_each_round(self):
        with patch.object(db, 'inspection', return_value=self.info), patch.object(db, 'ready'), patch.object(db, 'docker'), patch.object(db, 'stop_idle') as reset, patch.object(db, 'create') as create, patch.object(db.contract, 'load', return_value=self.policy):
            for _ in range(30):
                name, url = db.acquire(self.base)
                self.assertEqual(name, self.name)
                self.assertTrue(url.endswith(':54321/gate_test'))
                with self.assertRaisesRegex(ValueError, 'already leased'):
                    db.acquire(self.base)
                db.release(name)
            self.assertEqual(reset.call_count, 60)
            create.assert_not_called()
        self.assertEqual(db.LEASES, {})
        self.assertEqual([p.name for p in (self.base / 'db').iterdir()], ['primary.lock'])

    def test_lock_contention_missing_mount_and_alias_are_rejected(self):
        fd = db.lock_slot('primary')
        try:
            with self.assertRaises(BlockingIOError):
                db.lock_slot('primary')
        finally:
            os.close(fd)
        with patch.object(Path, 'is_mount', return_value=False):
            with self.assertRaisesRegex(ValueError, 'storage unavailable'):
                db.lock_slot('primary')
        link = self.base / 'alias'; link.symlink_to(self.base / 'db')
        with patch.object(db, 'DIRECTORY', link):
            with self.assertRaises(ValueError):
                db.lock_slot('primary')

    def test_database_contract_refuses_unrelated_or_unbounded_container(self):
        self.assertEqual(db.validate(self.info, self.name, self.policy)['memory'], 1024)
        variants = []
        for section, field, value in [('Config', 'Labels', {}), ('HostConfig', 'LogConfig', {}),
                                      ('HostConfig', 'Tmpfs', {}), ('HostConfig', 'Memory', 1)]:
            info = copy.deepcopy(self.info); info[section][field] = value; variants.append(info)
        info = copy.deepcopy(self.info); info['Mounts'] = [{'Type': 'volume'}]; variants.append(info)
        for info in variants:
            with self.assertRaises(ValueError): db.validate(info, self.name, self.policy)

    def test_inspection_distinguishes_absence_ambiguity_and_identity(self):
        with patch.object(db, 'docker', return_value=''):
            self.assertIsNone(db.inspection(self.name))
        with patch.object(db, 'docker', return_value='a\nb'):
            with self.assertRaises(ValueError): db.inspection(self.name)
        with patch.object(db, 'docker', side_effect=['identity', json.dumps([self.info])]):
            self.assertEqual(db.inspection(self.name), self.info)
        with patch.object(db.subprocess, 'check_output', return_value=' value\n'):
            self.assertEqual(db.docker('version'), 'value')

    def test_new_container_is_bounded_and_existing_stopped_slot_restarts(self):
        with patch.object(db, 'docker') as docker:
            db.create(self.name, self.policy)
        argv = docker.call_args.args
        self.assertIn('/var/lib/postgresql/data:rw,size=1024', argv)
        self.assertIn('--log-driver', argv)
        self.assertIn('codexsymphony.owner=' + db.OWNER, argv)
        info = copy.deepcopy(self.info); info['State']['Running'] = False
        with patch.object(db, 'inspection', side_effect=[None, info, self.info]), patch.object(db, 'create') as create, patch.object(db, 'ready'), patch.object(db, 'stop_idle'), patch.object(db, 'docker') as docker:
            db.provision(self.name, self.policy)
            create.assert_called_once_with(self.name, self.policy)
            docker.assert_called_once_with('start', self.name)
        info = copy.deepcopy(self.info); info['NetworkSettings']['Ports']['5432/tcp'][0]['HostIp'] = '0.0.0.0'
        with patch.object(db, 'inspection', return_value=info), patch.object(db, 'ready'), patch.object(db, 'stop_idle'), patch.object(db, 'docker'):
            with self.assertRaisesRegex(ValueError, 'publication changed'):
                db.provision(self.name, self.policy)

    def test_ready_failure_is_bounded(self):
        from types import SimpleNamespace
        with patch.object(db.subprocess, 'run', side_effect=[SimpleNamespace(returncode=1), SimpleNamespace(returncode=0)]), patch.object(db.time, 'sleep'):
            db.ready(self.name)
        with patch.object(db.subprocess, 'run', return_value=SimpleNamespace(returncode=1)) as run, patch.object(db.time, 'sleep'):
            with self.assertRaises(RuntimeError): db.ready(self.name)
            self.assertEqual(run.call_count, 60)

    def test_reset_refuses_clients_and_never_force_drops_database(self):
        with patch.object(db, 'psql', return_value='1') as psql:
            with self.assertRaises(ValueError): db.stop_idle(self.name)
            self.assertEqual(psql.call_count, 1)
        with patch.object(db, 'psql', return_value='0'), patch.object(db, 'docker') as docker:
            db.stop_idle(self.name)
            docker.assert_called_once_with('stop', '--time', '5', self.name)
        with patch.object(db, 'docker', return_value='0') as docker:
            self.assertEqual(db.psql(self.name, 'query'), '0')
            self.assertIn('ON_ERROR_STOP=1', docker.call_args.args)

    def test_failed_acquire_releases_lock_and_http_has_separate_slot(self):
        with patch.object(db.contract, 'load', return_value=self.policy), patch.object(db, 'provision', side_effect=RuntimeError('not ready')):
            with self.assertRaises(RuntimeError): db.acquire(self.base)
        fd = db.lock_slot('primary'); os.close(fd)
        with patch.object(db.contract, 'load', return_value=self.policy), patch.object(db, 'provision', return_value=({}, 'url')):
            name, _ = db.acquire(self.base, '-http')
            self.assertEqual(name, 'codexsymphony-gate-fixed-http')
        with patch.object(db, 'inspection', return_value=None):
            db.release(name)
        self.assertEqual(db.LEASES, {})

    def test_cleanup_error_releases_lease_and_exit_cleanup_is_best_effort(self):
        with patch.object(db.contract, 'load', return_value=self.policy), patch.object(db, 'provision', return_value=({}, 'url')):
            name, _ = db.acquire(self.base)
        with patch.object(db, 'inspection', return_value=self.info), patch.object(db, 'stop_idle', side_effect=RuntimeError('cleanup failed')):
            with self.assertRaises(RuntimeError): db.release(name)
        fd = db.lock_slot('primary'); os.close(fd)
        db.LEASES['fixture'] = -1
        with patch.object(db, 'release', side_effect=RuntimeError('exit error')):
            db.release_on_exit()
        db.LEASES.clear()


if __name__ == '__main__':
    unittest.main()
