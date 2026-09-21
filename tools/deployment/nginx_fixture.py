"""Execute the shipped ingress template with isolated paths and loopback ports."""
import subprocess
import time
import urllib.request
from pathlib import Path

from auth_contract_acceptance import ROOT, TLSCapture


class NginxFixture(TLSCapture):
    def start(self, address):
        root = Path(self.directory.name)
        port = self.server.server_port
        self.server.server_close()
        template = (ROOT / 'deploy/m2/nginx.conf').read_text()
        replacements = {
            '/run/codexsymphony-ingress/nginx.pid': str(root / 'nginx.pid'),
            'include /etc/nginx/mime.types;': 'default_type application/json;',
            '127.0.0.1:8443': f'127.0.0.1:{port}',
            '/etc/codexsymphony-ingress/fullchain.pem': str(root / 'cert.pem'),
            '/etc/codexsymphony-ingress/privkey.pem': str(root / 'key.pem'),
            '/opt/codexsymphony/web': str(root),
            '127.0.0.1:3081': address,
            'platform.example.invalid': '127.0.0.1',
        }
        for old, new in replacements.items():
            template = template.replace(old, new)
        template = template.replace('X-Forwarded-Host 127.0.0.1;',
                                    f'X-Forwarded-Host 127.0.0.1:{port};')
        # All writable Nginx paths belong to this disposable prefix.
        template = template.replace('http {', 'http {\n' + '\n'.join(
            f'    {name}_temp_path {root}/{name};'
            for name in ('client_body', 'proxy', 'fastcgi', 'uwsgi', 'scgi')))
        config = root / 'nginx.conf'
        config.write_text(template)
        self.log = open(root / 'nginx.log', 'w+b')
        command = ['nginx', '-p', str(root), '-c', str(config)]
        subprocess.run(command + ['-t'], stdout=self.log, stderr=self.log, check=True)
        self.process = subprocess.Popen(command + ['-g', 'daemon off;'],
                                        stdout=self.log, stderr=self.log)
        for _ in range(100):
            if self.process.poll() is not None:
                raise RuntimeError('fixture Nginx exited before readiness')
            try:
                with urllib.request.urlopen(self.origin + '/api/health',
                                            context=self.context, timeout=1) as response:
                    if response.status == 200:
                        return
            except OSError:
                time.sleep(.05)
        raise RuntimeError('fixture Nginx readiness timed out')

    def log_bytes(self):
        self.log.flush()
        self.log.seek(0)
        return self.log.read()

    def __exit__(self, *args):
        if hasattr(self, 'process') and self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=10)
        if hasattr(self, 'log'):
            self.log.close()
        super().__exit__(*args)
