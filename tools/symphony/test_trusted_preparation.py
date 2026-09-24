import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

path=Path(__file__).resolve().parents[1]/'preparation/app_server.py'
spec=importlib.util.spec_from_file_location('adapter',path)
adapter=importlib.util.module_from_spec(spec);spec.loader.exec_module(adapter)
probe_spec=importlib.util.spec_from_file_location('environment_probe',path.with_name('environment_probe.py'))
probe=importlib.util.module_from_spec(probe_spec);probe_spec.loader.exec_module(probe)

class TrustedPreparationTests(unittest.TestCase):
    def test_reviewed_project_environment_does_not_require_gate_or_rust(self):
        with patch.object(probe, 'run', return_value=probe.CODEX_VERSION) as run:
            self.assertEqual(probe.tools(project_verified=True), [])
            run.assert_called_once_with(['codex', '--version'])
        with patch.object(probe, 'run', return_value='wrong codex'):
            with self.assertRaisesRegex(ValueError, 'Codex version'):
                probe.tools(project_verified=True)

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
