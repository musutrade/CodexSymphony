import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('cleanup', Path(__file__).with_name('cleanup_issue_environments.py'))
cleanup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleanup)


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.journal = {'version': 1, 'entries': {'24': {
            'identifier': 'GH-24', 'phase': 'done',
            'handoff': {'repo': cleanup.REPOSITORY, 'merge_commit_sha': 'a' * 40}}}}
        self.resources = {
            'container': [{'Id': 'container-id', 'Name': '/codexsymphony-gh24-dev',
                           'Config': {'Labels': {cleanup.LABEL: 'GH-24-dev'}},
                           'Mounts': [{'Type': 'volume', 'Name': 'codexsymphony-gh24-dev-data',
                                       'Destination': '/var/lib/postgresql/data'}],
                           'NetworkSettings': {'Networks': {'codexsymphony-gh24-env': {}}}}],
            'volume': [{'Name': 'codexsymphony-gh24-dev-data', 'Labels': {cleanup.LABEL: 'GH-24'}}],
            'network': [{'Id': 'network-id', 'Name': 'codexsymphony-gh24-env',
                         'Internal': True, 'Labels': {cleanup.LABEL: 'GH-24'},
                         'Containers': {'container-id': {}}}]}

    def test_only_completed_merged_issue_is_eligible(self):
        self.assertTrue(cleanup.plan(self.journal, self.resources)[0]['eligible'])
        for phase in ['implementing', 'waiting', 'blocked', 'repair']:
            self.journal['entries']['24']['phase'] = phase
            self.assertFalse(cleanup.plan(self.journal, self.resources)[0]['eligible'])
        self.journal['entries']['24']['phase'] = 'done'
        self.journal['entries']['24']['handoff']['merge_commit_sha'] = ''
        self.assertFalse(cleanup.plan(self.journal, self.resources)[0]['eligible'])

    def test_unlabelled_product_database_and_other_projects_are_untouched(self):
        self.resources['container'].append({'Id': 'product', 'Name': '/codexsymphony-a01-gh24-db',
                                           'Config': {'Labels': {}}, 'Mounts': []})
        self.assertEqual(cleanup.plan(self.journal, self.resources)[0]['container'], ['container-id'])

    def test_unknown_volume_consumer_refuses_cleanup(self):
        other = copy.deepcopy(self.resources['container'][0])
        other.update(Id='other', Name='/unrelated')
        other['Config']['Labels'] = {}
        self.resources['container'].append(other)
        with self.assertRaisesRegex(ValueError, 'another container'):
            cleanup.plan(self.journal, self.resources)

    def test_unknown_network_consumer_refuses_cleanup(self):
        self.resources['network'][0]['Containers']['other'] = {}
        with self.assertRaisesRegex(ValueError, 'unrelated consumers'):
            cleanup.plan(self.journal, self.resources)

    def test_mislabelled_named_resource_refuses_cleanup(self):
        self.resources['volume'][0]['Labels'] = {}
        with self.assertRaisesRegex(ValueError, 'name/label conflict'):
            cleanup.plan(self.journal, self.resources)

    def test_bind_mount_refuses_cleanup(self):
        self.resources['container'][0]['Mounts'] = [{'Type': 'bind', 'Source': '/important'}]
        with self.assertRaisesRegex(ValueError, 'unexpected fixture mount'):
            cleanup.plan(self.journal, self.resources)

    def test_interrupted_removal_can_resume_with_only_volume_and_network(self):
        self.resources['container'] = []
        self.resources['network'][0]['Containers'] = {}
        item = cleanup.plan(self.journal, self.resources)[0]
        self.assertTrue(item['eligible'])
        self.assertEqual(item['container'], [])
        self.assertEqual(item['volume'], ['codexsymphony-gh24-dev-data'])
        self.assertEqual(cleanup.plan(self.journal, {k: [] for k in self.resources}), [])

    def test_journal_drift_before_apply_cannot_delete(self):
        item = cleanup.plan(self.journal, self.resources)[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'symphony').mkdir()
            self.journal['entries']['24']['phase'] = 'implementing'
            (root / 'symphony/WORKFLOW.lifecycle.md.handoffs.json').write_text(json.dumps(self.journal))
            with patch.object(cleanup, 'inventory', return_value=self.resources), patch.object(cleanup, 'run') as commands:
                with self.assertRaisesRegex(ValueError, 'plan changed'):
                    cleanup.apply_one(root, item)
                commands.assert_not_called()

    def test_invalid_journal_refuses_inventory_planning(self):
        with self.assertRaisesRegex(ValueError, 'invalid host'):
            cleanup.plan({'version': 2, 'entries': {}}, self.resources)


if __name__ == '__main__':
    unittest.main()
