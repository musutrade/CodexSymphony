import importlib.util,io,os,subprocess,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).parents[1]/'quality-host'))
from capture import wait_http_address

class ReadinessTests(unittest.TestCase):
 def child(self,code):
  p=subprocess.Popen([sys.executable,'-c',code],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
  self.addCleanup(self.stop,p);return p
 def stop(self,p):
  if p.poll() is None:p.terminate()
  p.wait(timeout=5);p.stdout.close()
 def test_ready_line_in_same_pipe_write_as_startup_logs(self):
  p=self.child('import os,time; os.write(1,b"startup log\\nAPI listening at http://127.0.0.1:43210\\n"); time.sleep(5)')
  out=io.BytesIO()
  self.assertEqual(wait_http_address(p,out,1),'127.0.0.1:43210')
  self.assertIn(b'startup log',out.getvalue())
 def test_ready_line_split_between_writes(self):
  p=self.child('import os,time; os.write(1,b"API listening at http://127."); time.sleep(.05); os.write(1,b"0.0.1:43210\\n"); time.sleep(5)')
  self.assertEqual(wait_http_address(p,io.BytesIO(),1),'127.0.0.1:43210')
 def test_exit_keeps_diagnostics(self):
  p=self.child('print("startup failed",flush=True)');out=io.BytesIO()
  with self.assertRaisesRegex(RuntimeError,'exited before readiness'):wait_http_address(p,out,1)
  self.assertIn(b'startup failed',out.getvalue())
 def test_timeout_keeps_diagnostics(self):
  p=self.child('import time; print("still starting",flush=True); time.sleep(5)');out=io.BytesIO()
  with self.assertRaisesRegex(RuntimeError,'readiness timeout'):wait_http_address(p,out,.2)
  self.assertIn(b'still starting',out.getvalue())
if __name__=='__main__':unittest.main()
