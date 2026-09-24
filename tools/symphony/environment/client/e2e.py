"""Start the workspace API, then run the repository's frontend E2E tests."""
import os, signal, subprocess, sys, time, urllib.request
from pathlib import Path

subprocess.run(['cargo','build','--locked','-p','codexsymphony-server'],check=True)
env=os.environ.copy();env['BIND_ADDRESS']='127.0.0.1:3081';env['WEB_ORIGIN']='http://127.0.0.1:4300'
log=Path('target/gh12-e2e-api.log').open('w')
server=subprocess.Popen(['target/debug/codexsymphony-server'],env=env,stdout=log,stderr=subprocess.STDOUT)
try:
    for _ in range(100):
        if server.poll() is not None: raise RuntimeError('API exited; see target/gh12-e2e-api.log')
        try:
            with urllib.request.urlopen('http://127.0.0.1:3081/api/health',timeout=1) as reply:
                if reply.status==200: break
        except OSError: pass
        time.sleep(.1)
    else: raise TimeoutError('API readiness failed')
    sys.exit(subprocess.call(['npm','run','test:e2e'],cwd='web/angular'))
finally:
    server.send_signal(signal.SIGINT)
    try: server.wait(timeout=5)
    except subprocess.TimeoutExpired: server.kill();server.wait()
    log.close()
