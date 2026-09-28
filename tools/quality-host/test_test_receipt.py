import json
from pathlib import Path
import tempfile
from unittest.mock import patch
import unittest
import test_execution_cache
import test_receipt as receipt


class FixedReceiptTests(test_execution_cache.CapturedTests):
    def fixture(self,root):
        repo,run,context=super().fixture(root)
        path=run/'probes/backend/bundle.json'
        value=json.loads(path.read_text());value['request']['parameters']['receipt']['pipeline']['capture']='cargo-llvm-cov-fixed-source/v1'
        path.write_text(json.dumps(value))
        return repo,run,context

    def test_changed_inputs_empty_test_log_and_noncanonical_receipt_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            repo,run,context=self.fixture(Path(directory))
            path=run/'probes/backend/bundle.json';value=json.loads(path.read_text())
            value['request']['parameters']['receipt']['inputs']={};path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError,'inputs differ'):receipt.seal(run,repo,context)
            value['request']['parameters']['receipt']['inputs']=receipt.backend_inputs(repo);path.write_text(json.dumps(value))
            (run/'probes/backend/capture.stdout').write_text('no passing tests')
            with self.assertRaisesRegex(ValueError,'no passing'):receipt.seal(run,repo,context)
            with self.assertRaisesRegex(ValueError,'noncanonical'):receipt.verified_log(Path('relative'),repo)


if __name__=='__main__':unittest.main()
