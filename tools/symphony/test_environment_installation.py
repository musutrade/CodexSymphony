"""Regression checks for host installation and per-Issue network allocation."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]

def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value)
    return value

install=module('install',ROOT/'tools/install_symphony_development.py')
provision=module('provision',Path(__file__).with_name('provision_issue_environment.py'))

class EnvironmentInstallationTests(unittest.TestCase):
    def test_reinstall_keeps_operator_labels_and_installs_new_hook(self):
        source=(ROOT/'WORKFLOW.lifecycle.md').read_bytes()
        old=source.replace(b'    - symphony-ready\n',b'    - symphony-ready\n    - operator-selected\n')
        result=install.routed_workflow(source,old)
        self.assertIn(b'    - operator-selected\n',result)
        self.assertNotIn(b'    - symphony-environment-acceptance\n',result)
        self.assertIn(b'provision_issue_environment.py "$PWD"',result)

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
