"""Actual isolated compiler cache: hit, invalidation, and private state."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'tools/quality-host'))
from isolation import command


class CompilerCache(unittest.TestCase):
    def test_hit_source_invalidation_and_run_isolation(self):
        with tempfile.TemporaryDirectory(prefix='sccache-test-') as temporary:
            run=Path(temporary)/'run';(run/'tmp/out').mkdir(parents=True)
            source=run/'tmp/lib.rs'
            source.write_text('pub fn answer() -> u32 { 42 }\n')

            def compile_in(directory):
                previous=set((directory/'tmp').glob('sccache-*.json'))
                args=command(['/home/gem/.cargo/bin/sccache','rustc','--crate-name','cache_probe',
                              '--crate-type','lib','--emit','link','--out-dir','/tmp/out','/tmp/lib.rs'],
                             run=directory,repository=ROOT,plugins=Path('/home/gem/.local/share/harness-gate'))
                subprocess.run(args,check=True,capture_output=True,timeout=60)
                paths=set((directory/'tmp').glob('sccache-*.json'))-previous
                self.assertEqual(len(paths),1)
                return json.loads(paths.pop().read_text())['stats']

            self.assertEqual(compile_in(run)['cache_misses']['counts']['Rust'],1)
            before=hashlib.sha256((run/'tmp/out/libcache_probe.rlib').read_bytes()).hexdigest()
            self.assertEqual(compile_in(run)['cache_hits']['counts']['Rust'],1)
            source.write_text('pub fn answer() -> u32 { 43 }\n')
            self.assertEqual(compile_in(run)['cache_misses']['counts']['Rust'],1)
            self.assertNotEqual(before,hashlib.sha256((run/'tmp/out/libcache_probe.rlib').read_bytes()).hexdigest())
            other=Path(temporary)/'other';(other/'tmp/out').mkdir(parents=True)
            (other/'tmp/lib.rs').write_bytes(source.read_bytes())
            self.assertEqual(compile_in(other)['cache_misses']['counts']['Rust'],1)


if __name__ == '__main__':
    unittest.main()
