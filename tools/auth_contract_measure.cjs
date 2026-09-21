// Local measurement with the installed pinned collector; no signed Gate verdict.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const root = process.argv[2];
const { measure } = require(path.join(process.argv[3], 'measure.cjs'));
const read = (name) => fs.readFileSync(path.join(root, name), 'utf8');
const sources = {};
for (const name of fs.readdirSync(path.join(root, 'web/angular/src'), { recursive: true })) {
  if (name.endsWith('.ts') && !name.endsWith('.spec.ts') && !name.endsWith('.d.ts')) {
    const file = `web/angular/src/${name}`;
    sources[file] = read(file);
  }
}
const result = measure(JSON.parse(read('api/baseline.json')), JSON.parse(read('api/openapi.json')),
  read('web/angular/src/app/health-response.ts'), read('web/angular/src/app/health.ts'), 'HealthResponse',
  JSON.parse(read('artifacts/gh71-product-contract/observations.json')), sources);
fs.writeFileSync(path.join(root, 'artifacts/gh71-product-contract/client-measurement.json'), JSON.stringify(result, null, 2)+'\n');
assert.equal(result['contract.compatible'].value, true, 'provider/client contract compatibility');
console.log('PASS: complete provider variants, unchanged baseline compatibility and actual Angular consumer AST');
