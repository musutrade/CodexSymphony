import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).parent/'environment'))
import runtime_product_acceptance as entry
import broker

class ReviewedRuntimeTests(unittest.TestCase):
    def test_fixed_entry_rejects_extra_arguments(self):
        for payload in ({'command':['/bin/sh']}, {'path':'/tmp/test'}, {'config':{}}):
            with self.assertRaises(ValueError):
                broker.perform({'action':'runtime-product-acceptance', **payload})

    def test_modified_added_and_symlinked_sources_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);root=base/'workspace';root.mkdir()
            reviewed=base/'reviewed-runtime';reviewed.mkdir()
            names=['Cargo.toml','Cargo.lock','rust-toolchain.toml','codex-version.lock']
            sources={}
            for name in names:
                (root/name).write_text('reviewed')
                sources[name]=hashlib.sha256(b'reviewed').hexdigest()
            (reviewed/'manifest.json').write_text(json.dumps({'workspace':str(root),'sources':sources}))
            with patch.object(entry,'BASE',base),patch.object(entry,'ROOT',root):
                entry.installation()
                (root/'Cargo.lock').write_text('changed')
                with self.assertRaisesRegex(ValueError,'source changed'):entry.installation()
                (root/'Cargo.lock').write_text('reviewed')
                extra=root/'apps/new.rs';extra.parent.mkdir();extra.write_text('new')
                with self.assertRaisesRegex(ValueError,'inventory changed'):entry.installation()
                extra.unlink()
                original=root/'Cargo.lock';original.unlink();original.symlink_to(root/'Cargo.toml')
                with self.assertRaisesRegex(ValueError,'symlink'):entry.installation()
