#!/usr/bin/python3
"""Read-only identity/health check for the already authorized A01/A13 deployment.

Does not claim Runtime execution or occupied-queue acceptance, merge a PR, or
release product ownership. Returns failure unless every pinned check passes.
"""
import json
import time
import urllib.error
import urllib.request


def check():
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def get(route):
        with opener.open('http://127.0.0.1:3081' + route, timeout=10) as response:
            return json.load(response)
    health = get('/api/health')
    if health.get('status') != 'ok' or health.get('database') != 'ok':
        raise RuntimeError('product health check failed')
    repositories = get('/api/multi/repository')['repositories']
    actual = {(r['repository']['remote'], r['repository']['github_repository_id'],
               r['repository']['base_branch'], r['delivery_ready']) for r in repositories}
    expected = {('musutrade/disposable', 1360824360, 'main', True),
                ('musutrade/disposable-2', 1377749969, 'main', True)}
    if actual != expected:
        raise RuntimeError('product repository identity or delivery configuration differs')


def main():
    # GitHub capability observations expire at 60s. Allow a bounded window for
    # the existing observer to refresh; this check never alters its state.
    for attempt in range(4):
        try:
            check()
            break
        except (RuntimeError, urllib.error.URLError):
            if attempt == 3:
                raise
            time.sleep(5)


if __name__ == '__main__':
    main()
