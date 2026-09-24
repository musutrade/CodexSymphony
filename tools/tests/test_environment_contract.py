import importlib.util
from pathlib import Path
import tempfile
import unittest
import os
import subprocess

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('contract',ROOT/'tools/environment_contract.py')
contract=importlib.util.module_from_spec(spec);spec.loader.exec_module(contract)


class ContractTests(unittest.TestCase):
    def test_temporary_probe_uses_declared_rust_even_outside_repository(self):
        policy=contract.load(ROOT)
        env=dict(os.environ,PATH=contract.tool_path(policy),**contract.test_environment(policy))
        with tempfile.TemporaryDirectory() as directory:
            selected=subprocess.check_output(['rustup','show','active-toolchain'],cwd=directory,env=env,text=True)
            self.assertTrue(selected.startswith(policy['tools']['rust']+'-'),selected)

    def test_checked_in_projections_match(self):
        contract.check_files(ROOT,contract.load(ROOT))

    def test_database_memory_swap_cpu_and_image_each_reject_drift(self):
        policy=contract.load(ROOT)
        state={'Image':policy['postgres']['image_id'], 'HostConfig':{'Memory':2147483648,'MemorySwap':4294967296,'NanoCpus':1000000000}}
        contract.check_database(state,policy)
        for key in state['HostConfig']:
            drift={'Image':state['Image'],'HostConfig':state['HostConfig']|{key:0}}
            with self.assertRaisesRegex(ValueError,'environment drift'):contract.check_database(drift,policy)
        with self.assertRaisesRegex(ValueError,'environment drift'):
            contract.check_database(state|{'Image':'different'},policy)

    def test_generated_lock_change_is_rejected(self):
        policy=contract.load(ROOT)
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            for name,body in contract.projections(policy).items():(root/name).write_text(body)
            (root/'.node-version').write_text('other\n')
            with self.assertRaisesRegex(ValueError,'.node-version'):contract.check_files(root,policy)


if __name__=='__main__':unittest.main()
