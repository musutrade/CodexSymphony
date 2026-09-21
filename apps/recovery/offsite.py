"""Explicit HTTPS object target, mTLS, verified readback, bounded retention."""
import hashlib
import http.client
import os
import ssl
import time
import urllib.parse
from common import private, require


def request(cfg, method, name, body=None):
    url = urllib.parse.urlsplit(cfg['url'])
    require(url.scheme == 'https' and url.hostname and not url.username and not url.password
            and not url.query and not url.fragment, 'credential-free HTTPS offsite target required')
    context = ssl.create_default_context(cafile=str(private(cfg['ca'])))
    context.load_cert_chain(str(private(cfg['certificate'])), str(private(cfg['private_key'])))
    connection = http.client.HTTPSConnection(url.hostname, url.port or 443, context=context, timeout=30)
    started = time.monotonic()
    try:
        headers = {'Content-Length': str(os.fstat(body.fileno()).st_size)} if body is not None else {}
        connection.request(method, url.path.rstrip('/') + '/' + name, body=body, headers=headers)
        response = connection.getresponse()
        require(response.status in ((200, 201, 204, 404) if method == 'DELETE' else (200, 201, 204)), 'offsite request failed; redirects forbidden')
        digest = hashlib.sha256()
        total = 0
        while block := response.read(1024 * 1024):
            require(time.monotonic() - started <= 120, 'offsite transfer deadline exceeded')
            total += len(block)
            require(total <= cfg['max_object_bytes'], 'offsite response exceeds limit')
            digest.update(block)
        return digest.hexdigest()
    finally:
        connection.close()


def transfer(cfg, archive, expected):
    require(archive.stat().st_size <= cfg['max_object_bytes'], 'offsite object exceeds limit')
    with archive.open('rb') as body:
        request(cfg, 'PUT', archive.name, body)
    require(request(cfg, 'GET', archive.name) == expected, 'offsite readback checksum mismatch')
    return 'fixture_https_verified' if cfg['fixture'] else 'offsite_https_verified'
