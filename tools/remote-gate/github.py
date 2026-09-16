"""Host-only GitHub App authentication and bounded REST transport."""
import base64
from datetime import datetime
import json
import subprocess
import time
import urllib.request
import urllib.error
import urllib.parse

API_ROOT = "https://api.github.com"


class RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # Tokens must never be forwarded to a redirect destination, including
        # same-origin URLs. Callers handle the HTTP failure without replaying writes.
        raise urllib.error.HTTPError(req.full_url, code, "GitHub API redirect rejected", headers, fp)


def encode(value):
    return base64.urlsafe_b64encode(value).rstrip(b'=')


def request(path, token, method='GET', body=None):
    parsed=urllib.parse.urlsplit(path)
    if not path.startswith('/') or path.startswith('//') or parsed.scheme or parsed.netloc or parsed.fragment or any(ord(c)<32 for c in path):
        raise ValueError('GitHub API path must be an absolute same-origin path')
    data=None if body is None else json.dumps(body).encode()
    req=urllib.request.Request(API_ROOT+path,data=data,method=method,
        headers={'Authorization':'Bearer '+token,'Accept':'application/vnd.github+json',
                 'Content-Type':'application/json','X-GitHub-Api-Version':'2022-11-28'})
    with urllib.request.build_opener(RejectRedirects()).open(req,timeout=45) as response:
        return json.load(response)


# Process-local only; the fixed permission scope below never changes.
_tokens={}

def installation_token(config):
    now=int(time.time())
    identity=(config['app_id'],config['app_key'],config['repository'])
    cached=_tokens.get(identity)
    if cached and cached[0]>now+60:
        return cached[1]
    payload=encode(b'{"alg":"RS256","typ":"JWT"}')+b'.'+encode(json.dumps(
        {'iat':now-60,'exp':now+300,'iss':str(config['app_id'])}).encode())
    signature=subprocess.check_output(['openssl','dgst','-sha256','-sign',config['app_key']],input=payload)
    jwt=(payload+b'.'+encode(signature)).decode()
    install=request('/repos/'+config['repository']+'/installation',jwt)
    if install['app_id']!=config['app_id']: raise ValueError('installation app mismatch')
    grant=request('/app/installations/'+str(install['id'])+'/access_tokens',jwt,'POST',
                  {'repositories':[config['repository'].split('/')[1]],
                   'permissions':{'checks':'write','contents':'read','actions':'read','pull_requests':'read'}})
    expires=int(datetime.fromisoformat(grant['expires_at'].replace('Z','+00:00')).timestamp())
    if expires<=now+60: raise ValueError('installation token expires too soon')
    # Reuse the same scoped grant across polling and processing. Repeated grants
    # in the same second use the same signed JWT and can fail upstream.
    _tokens[identity]=(min(expires,now+300),grant['token'])
    return grant['token']
