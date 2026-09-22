"""Regression checks for host installation and per-Issue network allocation."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import hashlib
import json

ROOT=Path(__file__).resolve().parents[2]

def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value)
    return value

install=module('install',ROOT/'tools/install_symphony_development.py')
provision=module('provision',Path(__file__).with_name('provision_issue_environment.py'))

class EnvironmentInstallationTests(unittest.TestCase):
    def test_install_persists_reviewed_gate_and_versions_release_with_it(self):
        with tempfile.TemporaryDirectory() as directory:
            home=Path(directory); source=home/'source'; state=home/'state/symphony'
            (source/'tools/symphony').mkdir(parents=True)
            (source/'WORKFLOW.lifecycle.md').write_bytes((ROOT/'WORKFLOW.lifecycle.md').read_bytes())
            for name in ('trusted_environment.py','reviewed_gate.py','provision_issue_environment.py','preserve_workspace.py'):
                (source/'tools/symphony'/name).write_bytes((ROOT/'tools/symphony'/name).read_bytes())
            (home/'.config/symphony').mkdir(parents=True)
            (home/'.config/symphony/codexsymphony.env').write_text('')
            (home/'.config/systemd/user').mkdir(parents=True)
            with patch.object(install,'HOME',home), patch.object(install,'BASE',home/'state'), patch.object(install,'ROOT',source), patch.object(install,'environment_sources',return_value={}), patch.object(install,'check_environment_resources'), patch.object(install,'check_preservation_controller'), patch.object(install.subprocess,'run'):
                install.main()
                original=(state/'codex-trusted').read_text()
                helper=(source/'tools/symphony/reviewed_gate.py').read_bytes()
                self.assertEqual((state/'reviewed_gate.py').read_bytes(),helper)
                releases=list((state/'releases').iterdir())
                self.assertEqual(len(releases),1)
                self.assertEqual((releases[0]/'reviewed_gate.py').read_bytes(),helper)
                self.assertIn(b'reviewed_gate.py)',(state/'WORKFLOW.lifecycle.md').read_bytes())
                (source/'tools/symphony/reviewed_gate.py').write_bytes(helper+b'\n# reviewed update\n')
                install.main()
                self.assertNotEqual((state/'codex-trusted').read_text(),original)
                self.assertEqual((releases[0]/'reviewed_gate.py').read_bytes(),helper)
                self.assertEqual((state/'reviewed_gate.py').read_bytes(),helper+b'\n# reviewed update\n')

    def test_required_preservation_cannot_install_against_unverified_controller(self):
        with tempfile.TemporaryDirectory() as directory:
            home=Path(directory); state=home/'state'; state.mkdir()
            binary=home/'symphony/elixir/bin/symphony'; binary.parent.mkdir(parents=True); binary.write_bytes(b'controller')
            marker=state/'preservation-controller.json'
            with patch.object(install,'HOME',home):
                with self.assertRaisesRegex(ValueError,'not installed'):
                    install.check_preservation_controller(state)
                marker.write_text(json.dumps({'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
                    'patch_sha256':hashlib.sha256((ROOT/'tools/symphony/controller-preservation.patch').read_bytes()).hexdigest()}))
                install.check_preservation_controller(state)
                binary.write_bytes(b'old-controller')
                with self.assertRaisesRegex(ValueError,'mismatch'):
                    install.check_preservation_controller(state)

    def test_reinstall_keeps_operator_labels_and_installs_new_hook(self):
        source=(ROOT/'WORKFLOW.lifecycle.md').read_bytes()
        old=source.replace(b'    - symphony-ready\n',b'    - symphony-ready\n    - operator-selected\n')
        result=install.routed_workflow(source,old)
        self.assertIn(b'    - operator-selected\n',result)
        self.assertNotIn(b'    - symphony-environment-acceptance\n',result)
        self.assertIn(b'provision_issue_environment.py "$PWD"',result)
        self.assertIn(b'before_remove_required: true',result)
        self.assertIn(b'preserve_workspace.py "$PWD"',result)

    def test_first_install_still_requires_environment_acceptance(self):
        self.assertIn(b'    - symphony-environment-acceptance\n',install.routed_workflow((ROOT/'WORKFLOW.lifecycle.md').read_bytes()))

    def test_malformed_existing_routing_is_rejected(self):
        with self.assertRaises(ValueError):
            install.routed_workflow((ROOT/'WORKFLOW.lifecycle.md').read_bytes(),b'unknown configuration')

    def test_missing_host_resources_fail_before_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError,'existing workflow has not been replaced'):
                install.check_environment_resources(Path(directory))

    def test_network_allocator_avoids_overlaps_and_is_independent_of_issue_number(self):
        networks=[{'IPAM':{'Config':[{'Subnet':'172.30.0.0/23'},{'Subnet':'172.30.2.0/28'},{'Subnet':'fd00::/64'}]}},
                  {'Name':'host','IPAM':{'Config':None}}]
        self.assertEqual(provision.available_subnet(networks),'172.30.3')

    def test_subnet_exhaustion_fails_explicitly(self):
        with self.assertRaisesRegex(RuntimeError,'No free fixture subnet'):
            provision.available_subnet([{'IPAM':{'Config':[{'Subnet':'172.30.0.0/16'}]}}])

if __name__=='__main__':unittest.main()
