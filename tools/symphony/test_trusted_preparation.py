import importlib.util
import json
from pathlib import Path
import unittest

path=Path(__file__).resolve().parents[1]/'preparation/app_server.py'
spec=importlib.util.spec_from_file_location('adapter',path)
adapter=importlib.util.module_from_spec(spec);spec.loader.exec_module(adapter)

class TrustedPreparationTests(unittest.TestCase):
    def test_failed_connectivity_remains_unavailable(self):
        config={'uid':1000,'workspace':'/project','deployment_identity':'v1'}
        sample={'uid':1000,'cwd':'/project','model_calls':0,'failures':[],
                'network':{'reachable':False}}
        result={'exitCode':0,'stdout':json.dumps(sample)}
        proof=adapter.evidence(config,{'type':'dangerFullAccess'},result)
        self.assertFalse(proof['network']['reachable'])
        self.assertEqual(proof['network']['configuration_identity'],'v1')
        self.assertNotIn('enforced',proof['network'])
        sample['cwd']='/other';result['stdout']=json.dumps(sample)
        with self.assertRaisesRegex(RuntimeError,'identity mismatch'):
            adapter.evidence(config,{},result)

    def test_command_failure_cannot_generate_success_evidence(self):
        with self.assertRaisesRegex(RuntimeError,'exit=7'):
            adapter.evidence({}, {}, {'exitCode':7,'stdout':'{}'})

if __name__=='__main__':unittest.main()
