"""Host-only GitHub App authentication and bounded REST transport."""
import base64
import json
import subprocess
import time
import urllib.request


def encode(value):
    return base64.urlsafe_b64encode(value).rstrip(b'=')


def request(path, token, method='GET', body=None):
    data=None if body is None else json.dumps(body).encode()
    req=urllib.request.Request('https://api.github.com'+path,data=data,method=method,
        headers={'Authorization':'Bearer '+token,'Accept':'application/vnd.github+json',
                 'Content-Type':'application/json','X-GitHub-Api-Version':'2022-11-28'})
    with urllib.request.urlopen(req,timeout=45) as response:
        return json.load(response)


def installation_token(config):
    now=int(time.time())
    payload=encode(b'{"alg":"RS256","typ":"JWT"}')+b'.'+encode(json.dumps(
        {'iat':now-60,'exp':now+300,'iss':str(config['app_id'])}).encode())
    signature=subprocess.check_output(['openssl','dgst','-sha256','-sign',config['app_key']],input=payload)
    jwt=(payload+b'.'+encode(signature)).decode()
    install=request('/repos/'+config['repository']+'/installation',jwt)
    if install['app_id']!=config['app_id']: raise ValueError('installation app mismatch')
    grant=request('/app/installations/'+str(install['id'])+'/access_tokens',jwt,'POST',
                  {'repositories':[config['repository'].split('/')[1]],
                   'permissions':{'checks':'write','contents':'read','actions':'read','pull_requests':'read'}})
    return grant['token']
