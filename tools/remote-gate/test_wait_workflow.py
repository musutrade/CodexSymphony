"""Execute the actual Actions waiter with a fake clock and authenticated checks."""
import json
from pathlib import Path
import subprocess
import textwrap
import unittest


class WaitBudgets(unittest.TestCase):
    def simulate(self, checks, queue='2', execution='2'):
        workflow = Path(__file__).resolve().parents[2] / '.github/workflows/quality.yml'
        script = textwrap.dedent(workflow.read_text().split('          script: |\n', 1)[1])
        fixture = {'script': script, 'checks': checks, 'queue': queue, 'execution': execution}
        runner = r'''
const q = JSON.parse(require('fs').readFileSync(0, 'utf8'));
let now = 0, calls = 0, failure;
const DateMock = class extends Date { static now() { return now; } };
const github = { rest: { checks: { listForRef: {} } }, paginate: async () => {
  const check = q.checks[Math.min(calls++, q.checks.length - 1)];
  if (!check) return [];
  return [{id: 7, head_sha:'sha', external_id:'1/1', app:{id:4867361,slug:'my-disposable-bot'}, ...check}];
}};
const core = { info: () => {}, setFailed: x => failure = x,
  summary: { addRaw: () => ({write: async () => {}}) }};
const context = {payload:{}, sha:'sha', runId:1, repo:{owner:'owner',repo:'repo'}};
const process = {env:{GITHUB_RUN_ATTEMPT:'1',GATE_QUEUE_MINUTES:q.queue,GATE_EXECUTION_MINUTES:q.execution}};
const timer = resolve => { now += 60000; resolve(); };
const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor;
new AsyncFunction('github','core','context','process','Date','setTimeout',q.script)
  (github,core,context,process,DateMock,timer).then(() => console.log(JSON.stringify({failure,calls,now})))
  .catch(error => console.log(JSON.stringify({error:error.message,calls,now})));
'''
        result = subprocess.check_output(['node', '-e', runner], input=json.dumps(fixture), text=True)
        return json.loads(result)

    def test_queue_time_does_not_consume_execution_budget(self):
        result = self.simulate([None, None, {'status': 'in_progress', 'started_at': '1970-01-01T00:02:00Z'},
                                {'status': 'completed', 'conclusion': 'success'}])
        self.assertNotIn('failure', result)
        self.assertEqual(result['now'], 180000)

    def test_separate_timeouts_and_wrong_app_cannot_start_clock(self):
        self.assertIn('queue wait', self.simulate([None])['failure'])
        self.assertIn('execution', self.simulate([{'status': 'in_progress', 'started_at': '1970-01-01T00:00:00Z'}])['failure'])
        self.assertIn('queue wait', self.simulate([{'status': 'in_progress', 'app': {'id': 123}}])['failure'])
        self.assertIn('Invalid GATE_QUEUE_MINUTES', self.simulate([None], queue='0')['error'])
        self.assertIn('invalid start time', self.simulate([{'status': 'in_progress', 'started_at': 'bad'}])['error'])
