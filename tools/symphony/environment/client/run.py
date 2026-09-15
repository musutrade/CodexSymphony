"""Run a command with PostgreSQL relays through the configured managed SOCKS proxy."""
import errno, os, select, socket, socketserver, struct, subprocess, sys, threading, time
from pathlib import Path
from urllib.parse import urlparse

def exact(s, count, deadline):
    result=b''
    while len(result)<count:
        s.settimeout(max(.001, deadline-time.monotonic()))
        part=s.recv(count-len(result))
        if not part: raise ConnectionError('SOCKS proxy closed connection')
        result+=part
    return result

def connect_once(host, timeout):
    deadline=time.monotonic()+timeout
    proxy=urlparse(os.environ.get('ALL_PROXY') or os.environ.get('all_proxy') or '')
    if not proxy.hostname: raise RuntimeError('Managed ALL_PROXY is missing')
    s=socket.create_connection((proxy.hostname,proxy.port),timeout)
    try:
        s.sendall(b'\x05\x01\x00')
        if exact(s,2,deadline)!=b'\x05\x00': raise PermissionError('SOCKS authentication rejected')
        s.sendall(b'\x05\x01\x00\x01'+socket.inet_aton(host)+struct.pack('!H',5432))
        reply=exact(s,4,deadline)
        if reply[0]!=5 or reply[2]!=0: raise ValueError('Invalid SOCKS reply')
        if reply[1] in (3,4,5,6): raise ConnectionRefusedError(errno.ECONNREFUSED, 'Database proxy transient rejection: '+str(reply[1]))
        if reply[1]!=0: raise PermissionError('Database proxy request rejected: '+str(reply[1]))
        size={1:4,4:16}.get(reply[3])
        if reply[3]==3: size=exact(s,1,deadline)[0]
        if size is None: raise ValueError('Invalid SOCKS address type')
        exact(s,size+2,deadline);s.settimeout(None)
        return s
    except BaseException:
        s.close();raise

def connect(host, timeout=30):
    """Retry only connection establishment; never replay SQL or a child command."""
    deadline=time.monotonic()+timeout
    attempts=0
    while True:
        attempts+=1
        try:
            return connect_once(host, min(5, max(.001, deadline-time.monotonic())))
        except OSError as error:
            transient=isinstance(error, (TimeoutError, ConnectionRefusedError, ConnectionResetError)) or error.errno in (errno.ENETUNREACH, errno.EHOSTUNREACH)
            if not transient: raise
            remaining=deadline-time.monotonic()
            if remaining<=0:
                raise TimeoutError(f'Fixture {host}:5432 unavailable after {attempts} connection attempts in {timeout}s; last error: {error}') from error
            print(f'Fixture {host}:5432 connection retry {attempts}: {error}',file=sys.stderr)
            time.sleep(min(remaining, .25*2**min(attempts-1,3)))

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
