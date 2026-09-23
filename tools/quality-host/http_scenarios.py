"""Data-only JSON request scenarios against the just-built isolated local API."""
import json
import http.cookiejar
import ssl
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


def capture(address, spec, scenarios, *, tls=None, variables=None, default_headers=None, setup_count=0, observer=None):
    if not re.fullmatch(r'127\.0\.0\.1:\d+', address):
        raise ValueError('capture requires the actual IPv4 loopback API listener')
    if not isinstance(scenarios, list) or len(scenarios) > 200:
        raise ValueError('bounded scenario list required')
    origin = tls.origin if tls else 'http://' + address
    if tls and (not origin.startswith('https://127.0.0.1:') or tls.context.verify_mode != ssl.CERT_REQUIRED or not tls.context.check_hostname):
        raise ValueError('verified loopback TLS required')
    cookies = http.cookiejar.CookieJar()
    handlers = [urllib.request.ProxyHandler({}), NoRedirect(), urllib.request.HTTPCookieProcessor(cookies)]
    if tls:
        handlers.append(urllib.request.HTTPSHandler(context=tls.context))
    opener = urllib.request.build_opener(*handlers)
    variables = variables or {}
    observed, responses = [], {}
    for index, step in enumerate(scenarios):
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
        headers = {'Origin': origin, 'Content-Type': 'application/json', 'x-codexsymphony-csrf': '1'}
        override = dict(default_headers or {}) if index >= setup_count else {}
        override.update(step.get('headers', {}))
        referenced_proofs = [value for key, value in override.items()
                             if key.lower() == 'x-codexsymphony-csrf'
                             and isinstance(value, dict) and '$response' in value]
        credentials = [resolve(value, responses) for value in referenced_proofs]
        override = resolve(capture_values(override, variables), responses)
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
        body = resolve(capture_values(step['body'], variables), responses) if 'body' in step else None
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
            if observer is not None:
                observer.observe(observation, setup=index < setup_count,
                                 record=step.get('record', True), cookies=[c.value for c in cookies],
                                 credentials=credentials)
            elif step.get('record', True):
                observed.append(observation)
    return observed


def capture_values(value, variables):
    """Only host-generated fixture values, never environment/file/shell interpolation."""
    if isinstance(value, dict):
        if '$capture' in value:
            if set(value) != {'$capture'} or value['$capture'] not in variables:
                raise ValueError('invalid capture variable')
            return variables[value['$capture']]
        return {k: capture_values(v, variables) for k, v in value.items()}
    if isinstance(value, list):
        return [capture_values(v, variables) for v in value]
    return value
