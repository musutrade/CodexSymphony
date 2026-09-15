"""Data-only JSON request scenarios against the just-built isolated local API."""
import json
import re
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        raise ValueError('API capture redirects are forbidden')


def resolve(value, responses):
    if isinstance(value, dict):
        if '$response' in value:
            if set(value) != {'$response'}:
                raise ValueError('invalid response reference')
            name, pointer = value['$response'].split('#', 1)
            current = responses[name]
            if pointer and not pointer.startswith('/'):
                raise ValueError('invalid JSON pointer')
            for part in pointer.split('/')[1:]:
                key = part.replace('~1', '/').replace('~0', '~')
                current = current[int(key)] if isinstance(current, list) else current[key]
            return current
        return {key: resolve(item, responses) for key, item in value.items()}
    if isinstance(value, list):
        return [resolve(item, responses) for item in value]
    return value


def capture(address, spec, scenarios):
    if not re.fullmatch(r'127\.0\.0\.1:\d+', address):
        raise ValueError('capture requires the actual IPv4 loopback API listener')
    if not isinstance(scenarios, list) or len(scenarios) > 200:
        raise ValueError('bounded scenario list required')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    observed, responses = [], {}
    for step in scenarios:
        allowed = {'id', 'method', 'path', 'status', 'path_parameters', 'body', 'headers', 'record'}
        if set(step) - allowed:
            raise ValueError('unknown capture scenario field')
        identity, method, path = step['id'], step['method'], step['path']
        if not re.fullmatch(r'[A-Za-z][A-Za-z0-9_-]*', identity) or identity in responses:
            raise ValueError('invalid/duplicate scenario identity')
        if method not in ['GET', 'POST', 'PUT', 'PATCH', 'DELETE']:
            raise ValueError('unsupported capture method')
        operation = spec['paths'].get(path, {}).get(method.lower())
        if operation is None or str(step['status']) not in operation['responses']:
            raise ValueError('scenario is outside the declared contract')
        if not path.startswith('/api/') or path.startswith('//') or any(c in path for c in ['?', '#', '\\']):
            raise ValueError('invalid API path')
        parameters = resolve(step.get('path_parameters', {}), responses)
        if set(parameters) != set(re.findall(r'\{(\w+)\}', path)):
            raise ValueError('path parameter mismatch')
        actual = path
        for name, value in parameters.items():
            if not isinstance(value, (str, int)) or isinstance(value, bool):
                raise ValueError('scalar path parameter required')
            value = str(value)
            if value in ['', '.', '..'] or any(c in value for c in ['/', '\\', '?', '#']):
                raise ValueError('invalid path parameter value')
            actual = actual.replace('{'+name+'}', urllib.parse.quote(value, safe=''))
        origin = 'http://' + address
        headers = {'Origin': origin, 'Content-Type': 'application/json', 'x-codexsymphony-csrf': '1'}
        override = step.get('headers', {})
        if any(key.lower() not in ['origin', 'x-codexsymphony-csrf', 'idempotency-key', 'if-match'] for key in override):
            raise ValueError('unsupported scenario header')
        for key, value in override.items():
            for old in list(headers):
                if old.lower() == key.lower():
                    del headers[old]
            if value is not None:
                if not isinstance(value, str) or '\r' in value or '\n' in value:
                    raise ValueError('invalid scenario header value')
                headers[key] = value
        body = resolve(step['body'], responses) if 'body' in step else None
        data = json.dumps(body).encode() if 'body' in step else None
        if data and len(data) > 1024 * 1024:
            raise ValueError('scenario request too large')
        request = urllib.request.Request(origin+actual, data=data, headers=headers, method=method)
        try:
            response = opener.open(request, timeout=8)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            if response.status != step['status']:
                raise ValueError(f'scenario {identity}: expected {step["status"]}, got {response.status}')
            raw = response.read(1024 * 1024 + 1)
            if len(raw) > 1024 * 1024:
                raise ValueError('scenario response too large')
            result = json.loads(raw) if raw else None
            responses[identity] = result
            observation = {'method': method, 'path': actual, 'status': response.status,
                           'content_type': response.headers.get('Content-Type', ''), 'body': result}
            if actual != path:
                observation['operation_path'] = path
            if 'body' in step:
                observation['request_body'] = body
            if step.get('record', True):
                observed.append(observation)
    return observed
