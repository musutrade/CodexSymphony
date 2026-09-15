import copy,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).parent/'environment'))
import broker,product_preparation_acceptance as acceptance

class ProductAcceptanceTests(unittest.TestCase):
    def test_fixed_dispatch_and_arbitrary_input_rejected(self):
        with patch.object(acceptance,'probe',return_value={'ok':True}) as probe:
            self.assertEqual(broker.perform({'action':'product-preparation-acceptance'}),{'ok':True})
            probe.assert_called_once_with()
        for extra in ({'command':['sh']},{'workspace':'/'},{'case':'ready'}):
            with self.assertRaises(ValueError):
                broker.perform({'action':'product-preparation-acceptance',**extra})

    def test_source_changes_and_symlinks_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp); root=base/'workspace'; directory=base/'client/reviewed-preparation'
            directory.mkdir(parents=True); (root/'tools/preparation').mkdir(parents=True)
            hashes={}
            for name in acceptance.FILES:
                (directory/name).write_text('reviewed')
                (root/'tools/preparation'/name).write_text('reviewed')
                hashes[name]=acceptance.digest(directory/name)
            (directory/'manifest.json').write_text(json.dumps(hashes))
            with patch.object(acceptance,'BASE',base),patch.object(acceptance,'ROOT',root):
                self.assertEqual(acceptance.reviewed_sources()[1],hashes)
                source=root/'tools/preparation/app_server.py'
                source.write_text('changed')
                with self.assertRaisesRegex(ValueError,'changed'):acceptance.reviewed_sources()
                source.unlink();source.symlink_to(directory/'app_server.py')
                with self.assertRaisesRegex(ValueError,'symlink'):acceptance.reviewed_sources()

    def test_missing_boundary_or_unexpected_failure_cannot_pass(self):
        result={'execution':{'model_calls':0},'network':{'allowed_probe':True,'denied_probe':True,
                'direct_connection_rejected':True,'enforced':True,'configuration_identity':'known'},'failures':[]}
        acceptance.validate(result)
        for key in ('allowed_probe','denied_probe','direct_connection_rejected','enforced'):
            broken=copy.deepcopy(result);broken['network'][key]=False
            with self.assertRaises(ValueError):acceptance.validate(broken)
        with self.assertRaises(ValueError):acceptance.validate(result,'preparation_dependency_missing')
        with self.assertRaises(ValueError):acceptance.validate(result,unknown=True)
        result['execution']['model_calls']=1
        with self.assertRaises(ValueError):acceptance.validate(result)

if __name__=='__main__':unittest.main()
