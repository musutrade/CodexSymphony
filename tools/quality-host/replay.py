"""Host-owned durable nonce claims for Core verify's process-local replay guard."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import socket
import threading
from capture import load

FIELDS=('nonce','invocation_id','step_id','config_digest')

def identity(request): return {name:request[name] for name in FIELDS}

@contextmanager
def broker(run, ledger):
    ledger.mkdir(parents=True,exist_ok=True,mode=0o700)
    allowed={}
    for path in (run/'workspace/.harness-gate/runtime').glob('*-request.json'):
        request=load(path);allowed[request['nonce']]=identity(request)
    address=run/'nonce.sock'
    if address.exists(): address.unlink()
    server=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);server.bind(str(address));address.chmod(0o600);server.listen();server.settimeout(.2)
    stopped=threading.Event()
    def serve():
        while not stopped.is_set():
            try: connection,_=server.accept()
            except socket.timeout: continue
            except OSError: return
            with connection:
                connection.settimeout(2)
                try:
                    data=b''
                    while not data.endswith(b'\n') and len(data)<4096:
                        part=connection.recv(4096-len(data))
                        if not part: break
                        data+=part
                    value=json.loads(data)
                    nonce=value['nonce']
                    if value!=allowed.get(nonce): raise ValueError('unapproved nonce claim')
                    digest=hashlib.sha256(nonce.encode()).hexdigest()
                    marker=ledger/('nonce-'+digest+'.json')
                    descriptor=os.open(marker,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
                    with os.fdopen(descriptor,'w') as output:
                        json.dump(value,output);output.flush();os.fsync(output.fileno())
                    directory=os.open(ledger,os.O_RDONLY|os.O_DIRECTORY)
                    try: os.fsync(directory)
                    finally: os.close(directory)
                    response={'accepted':True}
                except FileExistsError: response={'accepted':False,'reason':'nonce has already been used'}
                except (ValueError,KeyError,TypeError,OSError) as error: response={'accepted':False,'reason':str(error)}
                with (run/'nonce-events.jsonl').open('a') as log: log.write(json.dumps(response)+'\n')
                try: connection.sendall((json.dumps(response)+'\n').encode())
                except OSError: pass
    thread=threading.Thread(target=serve,daemon=True);thread.start()
    try: yield
    finally:
        stopped.set();server.close();thread.join(timeout=3);address.unlink(missing_ok=True)
