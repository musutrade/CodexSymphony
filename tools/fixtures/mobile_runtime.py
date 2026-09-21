"""Scripted app-server transport for the isolated B03 service/browser test.

This is not the pinned Codex binary or a model acceptance claim. The real service
owns launches, process stop receipts, Git preservation, questions and budgets.
Preparation command/exec runs the actual supplied environment probe.
"""
import json
import os
from pathlib import Path
import subprocess
import sys


THREAD = 'thread-' + str(os.getpid())

def send(value):
    print(json.dumps(value), flush=True)


def tool(identity, name, arguments):
    send({'id': identity, 'method': 'item/tool/call', 'params': {
        'threadId': THREAD, 'turnId': 'turn', 'callId': str(identity),
        'tool': name, 'arguments': arguments}})


for line in sys.stdin:
    request = json.loads(line)
    method, params = request.get('method'), request.get('params', {})
    identity = request.get('id')
    if method == 'initialize':
        send({'id': identity, 'result': {'userAgent': 'scripted-mobile-fixture/0.154.0 (test)'}})
    elif method == 'command/exec':
        result = subprocess.run(params['command'], cwd=params['cwd'], capture_output=True, text=True, timeout=80)
        send({'id': identity, 'result': {'exitCode': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}})
    elif method == 'thread/start':
        send({'id': identity, 'result': {'cwd': os.getcwd(), 'thread': {'id': THREAD, 'cwd': os.getcwd()}}})
    elif method == 'turn/start':
        saved = json.loads(params['input'][0]['text'])
        Path('received-input.json').write_text(json.dumps(saved))
        Path('executed-by.json').write_text(json.dumps({'pid': os.getpid(), 'cwd': os.getcwd()}))
        send({'id': identity, 'result': {'turn': {'id': 'turn'}}})
        send({'method': 'thread/tokenUsage/updated', 'params': {'threadId': THREAD, 'turnId': 'turn', 'tokenUsage': {
            'total': {'inputTokens': 20, 'cachedInputTokens': 5, 'outputTokens': 10},
            'last': {'inputTokens': 20, 'cachedInputTokens': 5, 'outputTokens': 10}}}})
        if saved['confirmed_answers']:
            assert Path('paid-work.txt').read_text() == 'original unfinished work\n'
            Path('resumed-proof.json').write_text(json.dumps(saved['confirmed_answers']))
            # Remain active until the browser pauses; later resumes retain this file.
        else:
            Path('paid-work.txt').write_text('original unfinished work\n')
            send({'id': 'business-rpc', 'method': 'item/tool/requestUserInput', 'params': {
                'threadId': THREAD, 'turnId': 'turn', 'itemId': 'choice', 'isBlocking': True,
                'questions': [{'id': 'choice', 'question': '隔夜继续使用原范围？', 'options': [{'label': '继续'}]}]}})
    elif method == 'turn/interrupt':
        send({'id': identity, 'result': {}})
