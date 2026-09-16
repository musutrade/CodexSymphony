import importlib.util,sys,unittest
from pathlib import Path
from unittest.mock import patch,MagicMock
BASE=Path(__file__).parent/'environment'
sys.path.insert(0,str(BASE))
import broker,execution_readiness

class ReadinessTests(unittest.TestCase):
 def test_fixed_operation_uses_only_host_probe(self):
  with patch.object(execution_readiness,'probe',return_value={'ok':True}) as probe:
   self.assertEqual(broker.perform({'action':'execution-readiness'}),{'ok':True})
   probe.assert_called_once_with()
 def test_arguments_cannot_select_command_or_workspace(self):
  for extra in [{'command':['sh']},{'workspace':'/'},{'role':'dev'}]:
   with self.assertRaises(ValueError):broker.perform({'action':'execution-readiness',**extra})
 def test_failed_inner_command_cannot_be_success(self):
  with patch.object(execution_readiness,'execute',return_value={'command_exec':{'result':{'exitCode':1,'stderr':'denied'}}}):
   with self.assertRaisesRegex(RuntimeError,'denied'):execution_readiness.probe()
 def test_missing_network_policy_cannot_be_success(self):
  with patch.object(execution_readiness,'execute',return_value={'command_exec':{'result':{'exitCode':0}},'requirements':{'requirements':None}}):
   with self.assertRaisesRegex(RuntimeError,'network'):execution_readiness.probe()
if __name__=='__main__':unittest.main()
