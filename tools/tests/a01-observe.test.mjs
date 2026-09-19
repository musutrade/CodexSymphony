// Deterministic HTTP fixtures, not real A01 evidence.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { observeDelivery, resumeEvidence } from '../a01-observe.mjs';

test('native fetch continuation preserves the Ready identity and only reads', async t => {
  const requests = [];
  const input = { repository: 'owner/repo', repository_id: 123, timeout_seconds: 0.02 };
  const saved = { schema: 'codexsymphony-a01/v1', mode: 'real', repository: input.repository,
    repository_id: 123, requirement_id: 1, revision: 1, ready: { revision: 1 } };
  let operations = { requirement: { id: 1, revision: 1, state: 'Submitted' } };
  let deliveries = [{ revision: 1, repository: input.repository, repository_id: 123,
    head_sha: 'a'.repeat(40), pr_number: 4, validation_id: 'validation-1' }];
  let status = 200;
  const server = createServer((request, response) => {
    requests.push([request.method, request.url]);
    response.writeHead(status, { 'content-type': 'application/json' });
    response.end(JSON.stringify(request.url === '/api/repository'
      ? { repositories: [{ repository: { remote: input.repository, github_repository_id: 123, revoked: false } }] }
      : request.url.endsWith('/operations') ? operations : { deliveries }));
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  t.after(() => new Promise(done => server.close(done)));
  input.origin = `http://127.0.0.1:${server.address().port}`;
  const evidence = () => ({ ...resumeEvidence(saved, input), status: 'started' });
  const persisted = [];
  const result = evidence();
  await observeDelivery(input, result, async () => persisted.push(structuredClone(result)));
  assert.equal(result.status, 'submitted');
  assert.equal(result.operations.requirement.state, 'Submitted');
  assert.equal(result.delivery.validation_id, 'validation-1');
  assert.equal(persisted.at(-1).pr_url, 'https://github.com/owner/repo/pull/4');
  assert.throws(() => resumeEvidence({ ...saved, repository_id: 999 }, input), /saved Ready identity/);
  assert.throws(() => resumeEvidence({ ...saved, revision: null }, input), /saved Ready identity/);
  operations.requirement.revision = 2;
  await assert.rejects(observeDelivery(input, evidence(), async () => {}), /revision differs/);
  operations.requirement.revision = 1;
  deliveries.push({ ...deliveries[0], pr_number: 5 });
  await assert.rejects(observeDelivery(input, evidence(), async () => {}), /multiple PR/);
  deliveries = [{ ...deliveries[0], head_sha: 'wrong' }];
  await assert.rejects(observeDelivery(input, evidence(), async () => {}), /identity differs/);
  deliveries = [];
  operations.storage = { blocked: true, error: 'retained scan failure' };
  await assert.rejects(observeDelivery(input, evidence(), async () => {}), /retained scan failure/);
  delete operations.storage;
  await assert.rejects(observeDelivery(input, evidence(), async () => {}), /bounded smoke wait expired/);
  status = 503;
  await assert.rejects(observeDelivery(input, evidence(), async () => {}), /observation returned 503/);
  assert(requests.every(([method]) => method === 'GET'));
  assert(requests.every(([, path]) => path === '/api/repository' || path.startsWith('/api/requirements/1/')));
});
