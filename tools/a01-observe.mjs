// Read-only continuation of the explicitly authorized real A01 smoke.
export function resumeEvidence(previous, input) {
  if (previous.schema !== 'codexsymphony-a01/v1' || previous.mode !== 'real' ||
      previous.repository !== input.repository || previous.repository_id !== input.repository_id ||
      !Number.isSafeInteger(previous.requirement_id) || previous.requirement_id <= 0 ||
      !Number.isSafeInteger(previous.revision) || previous.revision <= 0 ||
      previous.ready?.revision !== previous.revision) {
    throw new Error('saved Ready identity required; retain original and reconcile');
  }
  return { requirement_id: previous.requirement_id, revision: previous.revision, ready: previous.ready };
}

async function readJson(origin, path) {
  const response = await fetch(new URL(path, origin), { signal: AbortSignal.timeout(10000), redirect: 'error' });
  if (!response.ok) throw new Error(`${path} observation returned ${response.status}`);
  return response.json();
}

export async function observeDelivery(input, evidence, save) {
  const seconds = input.timeout_seconds ?? 900;
  if (!Number.isFinite(seconds) || seconds <= 0 || seconds > 1800) throw new Error('timeout_seconds must be in (0,1800]');
  const registered = await readJson(input.origin, '/api/repository');
  if (!registered.repositories?.some(r => r.repository.remote === input.repository &&
      r.repository.github_repository_id === input.repository_id && !r.repository.revoked)) {
    throw new Error('designated repository is not configured');
  }
  const path = `/api/requirements/${evidence.requirement_id}`;
  const deadline = Date.now() + seconds * 1000;
  while (Date.now() < deadline) {
    evidence.operations = await readJson(input.origin, `${path}/operations`);
    const current = evidence.operations.requirement;
    if (current?.id !== evidence.requirement_id || current.revision !== evidence.revision) {
      throw new Error('current Requirement revision differs; reconcile original');
    }
    const { deliveries } = await readJson(input.origin, `${path}/delivery`);
    evidence.deliveries = deliveries;
    await save();
    const submitted = deliveries.filter(d => d.revision === evidence.revision && d.pr_number);
    if (new Set(submitted.map(d => d.pr_number)).size > 1) throw new Error('multiple PR identities; reconcile');
    const delivery = submitted[0];
    if (delivery) {
      if (delivery.repository !== input.repository || delivery.repository_id !== input.repository_id ||
          !/^[a-f0-9]{40}$/.test(delivery.head_sha) || !delivery.validation_id) throw new Error('delivery identity differs');
      evidence.delivery = delivery;
      evidence.status = 'submitted';
      evidence.pr_url = `https://github.com/${delivery.repository}/pull/${delivery.pr_number}`;
      await save();
      return;
    }
    if (evidence.operations.storage?.blocked) throw new Error(`storage blocked: ${evidence.operations.storage.error}; retain original and reconcile`);
    await new Promise(done => setTimeout(done, Math.min(5000, Math.max(0, deadline - Date.now()))));
  }
  throw new Error('bounded smoke wait expired; retain requirement and reconcile, do not recreate');
}
