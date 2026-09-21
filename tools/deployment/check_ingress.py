#!/usr/bin/env python3
"""Read-only HTTPS/origin check; never logs cookies, passwords or response bodies."""
import argparse
from contextlib import closing
import http.client
import ipaddress
import json
from pathlib import Path
import ssl
import urllib.parse


def remote(origin, ca=None):
    url = urllib.parse.urlsplit(origin)
    if url.scheme != 'https' or url.path or url.query or url.fragment or url.username:
        raise ValueError('an exact HTTPS origin is required')
    context = ssl.create_default_context(cafile=ca)
    statuses = []
    for headers in ({}, {'CF-Access-Jwt-Assertion': 'forged',
                         'CF-Access-Authenticated-User-Email': 'fixture@example.invalid'},
                    {'CF-Access-Jwt-Assertion': 'eyJhbGciOiJSUzI1NiJ9.eyJlbWFpbCI6ImZpeHR1cmVAZXhhbXBsZS5pbnZhbGlkIn0.fixture'}):
        with closing(http.client.HTTPSConnection(url.hostname, url.port or 443, context=context, timeout=10)) as connection:
            connection.request('GET', '/api/drafts', headers=headers)
            response = connection.getresponse()
            statuses.append(response.status)
            response.read()
            if response.status != 401:
                raise ValueError('unauthenticated HTTPS business request was not rejected with 401')
    return {'verified_https': True, 'anonymous_access_headers_status': statuses}


def origin_listener(pid, backend):
    host, port = backend.split(':')
    if not ipaddress.ip_address(host).is_loopback:
        raise ValueError('origin must listen only on loopback')
    sockets = set()
    for fd in (Path('/proc')/str(pid)/'fd').iterdir():
        try:
            link = str(fd.readlink())
        except FileNotFoundError:
            continue
        if link.startswith('socket:['):
            sockets.add(link[8:-1])
    listeners = []
    for family in ('tcp', 'tcp6'):
        for line in (Path('/proc')/str(pid)/'net'/family).read_text().splitlines()[1:]:
            fields = line.split()
            if fields[3] == '0A' and fields[9] in sockets:
                listeners.append((family, fields[1]))
    expected = ('tcp', f'{int(ipaddress.IPv4Address(host)):08X}'[6:8] +
                f'{int(ipaddress.IPv4Address(host)):08X}'[4:6] +
                f'{int(ipaddress.IPv4Address(host)):08X}'[2:4] +
                f'{int(ipaddress.IPv4Address(host)):08X}'[0:2] + f':{int(port):04X}')
    if listeners != [expected]:
        raise ValueError('origin PID has missing, additional or non-loopback listeners')
    with closing(http.client.HTTPConnection(host, int(port), timeout=5)) as connection:
        connection.request('GET', '/api/drafts')
        response = connection.getresponse()
        if response.status not in (401, 403):
            raise ValueError('direct origin business access was not rejected')
        response.read()
    return {'origin_pid_owned_loopback_only': True, 'direct_origin_rejected': True}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--origin', required=True)
    parser.add_argument('--ca')
    parser.add_argument('--pid', type=int)
    parser.add_argument('--backend')
    args = parser.parse_args()
    result = remote(args.origin, args.ca)
    if args.pid or args.backend:
        if not (args.pid and args.backend):
            parser.error('--pid and --backend must be supplied together')
        result.update(origin_listener(args.pid, args.backend))
    print(json.dumps(result))
