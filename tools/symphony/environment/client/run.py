"""Run a command with PostgreSQL relays through the configured managed SOCKS proxy."""
import os, select, socket, socketserver, struct, subprocess, sys, threading
from pathlib import Path
from urllib.parse import urlparse

def exact(s, count):
    result=b''
    while len(result)<count:
        part=s.recv(count-len(result))
        if not part: raise ConnectionError('SOCKS proxy closed connection')
        result+=part
    return result

def connect(host):
    proxy=urlparse(os.environ.get('ALL_PROXY') or os.environ.get('all_proxy') or '')
    if not proxy.hostname: raise RuntimeError('Managed ALL_PROXY is missing')
    s=socket.create_connection((proxy.hostname,proxy.port),10)
    try:
        s.sendall(b'\x05\x01\x00')
        if exact(s,2)!=b'\x05\x00': raise ConnectionError('SOCKS authentication rejected')
        s.sendall(b'\x05\x01\x00\x01'+socket.inet_aton(host)+struct.pack('!H',5432))
        reply=exact(s,4)
        if reply[1]!=0: raise ConnectionError('Database proxy request rejected: '+str(reply[1]))
        size={1:4,4:16}.get(reply[3])
        if reply[3]==3: size=exact(s,1)[0]
        exact(s,size+2);s.settimeout(None)
        return s
    except BaseException:
        s.close();raise

class Relay(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            with connect(self.server.database_host) as upstream:
                sockets=[self.request,upstream]
                while True:
                    ready,_,_=select.select(sockets,[],[],60)
                    if not ready: continue
                    for source in ready:
                        data=source.recv(65536)
                        if not data: return
                        (upstream if source is self.request else self.request).sendall(data)
        except (OSError,ConnectionError) as error:
            print('Database relay: '+str(error),file=sys.stderr)

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address=True
    daemon_threads=True

def main():
    servers=[]
    try:
        env=os.environ.copy()
        env['PATH']='/opt/gh12-env/bin:'+env.get('PATH','/usr/bin:/bin')
        # Only loopback inside this command's isolated network namespace is local.
        # Managed proxy routing still governs all connections leaving the sandbox.
        env['NO_PROXY']=env['no_proxy']='127.0.0.1,localhost,::1'
        for role,host,port in [('test','172.30.212.2',54329),('dev','172.30.212.3',54330)]:
            with connect(host): pass
            server=Server(('127.0.0.1',port),Relay);server.database_host=host
            servers.append(server);threading.Thread(target=server.serve_forever,daemon=True).start()
            user='codexsymphony_'+role
            env['TEST_DATABASE_URL' if role=='test' else 'DEV_DATABASE_URL']=f'postgres://{user}:{user}@127.0.0.1:{port}/{user}?sslmode=disable'
        env['DATABASE_URL']=env['TEST_DATABASE_URL']
        command=sys.argv[1:]
        if not command: raise SystemExit('usage: python3 /opt/gh12-env/run.py COMMAND [ARGS]')
        if command[0]=='psql':
            pg=Path(__file__).parent/'pg'
            command=[str(pg/'ld-musl-x86_64.so.1'),'--library-path',str(pg),str(pg/'psql'),*command[1:]]
        return subprocess.call(command,env=env)
    finally:
        for server in servers: server.shutdown();server.server_close()

if __name__=='__main__': sys.exit(main())
