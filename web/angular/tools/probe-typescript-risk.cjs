// Local acceptance probe only: does not issue signed production gate evidence.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const { execFileSync } = require('node:child_process');
const { createInstrumenter } = require('istanbul-lib-instrument');

const root = path.resolve(__dirname, '..');
const plugin = process.env.HARNESS_GATE_TYPESCRIPT_PLUGIN;
assert(
  plugin && path.isAbsolute(plugin),
  'Set HARNESS_GATE_TYPESCRIPT_PLUGIN to the installed package directory',
);
const { measure } = require(path.join(plugin, 'measure.cjs'));
const output = fs.mkdtempSync(path.join(os.tmpdir(), 'codexsymphony-ts-risk-'));
console.log(`Retaining probe inputs and results: ${output}`);
const app = path.join(output, 'app');
const captures = path.join(output, 'captures');
fs.mkdirSync(captures);
const sourceRoot = path.join(output, 'source');
fs.cpSync(root, sourceRoot, {
  recursive: true,
  filter: (name) =>
    !['node_modules', '.angular', 'dist', 'test-results', 'playwright-report', '.git'].includes(
      path.basename(name),
    ),
});
fs.cpSync(sourceRoot, app, { recursive: true });
fs.symlinkSync(path.join(root, 'node_modules'), path.join(app, 'node_modules'), 'dir');
const read = (name) => fs.readFileSync(name, 'utf8');
const write = (name, value) =>
  fs.writeFileSync(path.join(output, name), JSON.stringify(value, null, 2) + '\n');
const sha = (value) => createHash('sha256').update(value).digest('hex');
const sources = fs
  .readdirSync(path.join(sourceRoot, 'src'), { recursive: true })
  .filter((name) => name.endsWith('.ts') && !name.endsWith('.d.ts'))
  .sort();
const excluded = sources.filter((name) => name.endsWith('.spec.ts'));
const originals = {};
const baseline = {};
for (const name of sources.filter((name) => !excluded.includes(name))) {
  const file = path.join(app, 'src', name);
  const source = read(file);
  const instrumenter = createInstrumenter({
    parserPlugins: ['typescript', 'decorators'],
    produceSourceMap: false,
  });
  fs.writeFileSync(file, instrumenter.instrumentSync(source, file));
  baseline[file] = instrumenter.lastFileCoverage();
  originals[file] = { path: `src/${name}`, text: source, sha256: sha(source) };
}
write('originals.json', originals);
write('baseline.json', baseline);
const config = JSON.parse(read(path.join(app, 'angular.json')));
config.projects['codexsymphony-web'].architect.test.options = {
  setupFiles: ['./capture.setup.ts'],
};
fs.writeFileSync(path.join(app, 'angular.json'), JSON.stringify(config, null, 2));
fs.writeFileSync(
  path.join(app, 'tsconfig.spec.json'),
  JSON.stringify({
    extends: './tsconfig.json',
    compilerOptions: { outDir: './out-tsc/spec', types: ['vitest/globals', 'node'] },
    include: ['src/**/*.d.ts', 'src/**/*.spec.ts', 'capture.setup.ts'],
  }),
);
fs.writeFileSync(
  path.join(app, 'capture.setup.ts'),
  `
import { afterAll } from 'vitest';
import { writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
afterAll(() => {
  const counters = (globalThis as unknown as { __coverage__?: unknown }).__coverage__ ?? {};
  writeFileSync(${JSON.stringify(captures)} + '/' + randomUUID() + '.json', JSON.stringify(counters));
});
`,
);
execFileSync('npm', ['test', '--', '--watch=false'], { cwd: app, stdio: 'inherit' });
const snapshots = fs.readdirSync(captures).sort();
assert(snapshots.length > 0, 'No captured counters');
for (const name of snapshots) {
  for (const [file, native] of Object.entries(JSON.parse(read(path.join(captures, name))))) {
    const target = baseline[file];
    assert(target, `Uninventoried source: ${file}`);
    for (const map of ['statementMap', 'fnMap', 'branchMap'])
      assert.deepEqual(native[map], target[map]);
    for (const counter of ['s', 'f', 'b']) {
      assert.deepEqual(Object.keys(native[counter]).sort(), Object.keys(target[counter]).sort());
      for (const key of Object.keys(target[counter])) {
        const previous = target[counter][key];
        const next = native[counter][key];
        const valid = (n) => Number.isSafeInteger(n) && n >= 0;
        if (Array.isArray(previous)) {
          assert(Array.isArray(next) && previous.length === next.length && next.every(valid));
          target[counter][key] = previous.map((n, i) => Math.max(n, next[i]));
        } else {
          assert(valid(next));
          target[counter][key] = Math.max(previous, next);
        }
      }
    }
  }
}
write('coverage.json', baseline);
const result = Object.entries(originals).map(([file, source]) =>
  measure(source.path, source.text, baseline[file]),
);
write('measurement.json', result);
// A separate original-source snapshot is consumed by the host-side signed
// transport rehearsal. Test execution never receives a signing key.
fs.writeFileSync(path.join(sourceRoot, '.local-capture-snapshot'), 'frontend-only rehearsal\n');
fs.writeFileSync(path.join(sourceRoot, 'coverage.json'), JSON.stringify(baseline));
fs.mkdirSync(path.join(sourceRoot, 'evidence'));
const protocol = require(path.join(plugin, 'protocol.cjs'));
const revision = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
const request = {
  schema: 'harness-collector-request/v1',
  project: 'codexsymphony',
  component: 'frontend',
  collector: protocol.COLLECTOR,
  context: { commit: revision, base_commit: revision, target: 'node', run: path.basename(output) },
  requested_capabilities: [
    'complexity.cyclomatic',
    'coverage.line',
    'coverage.function',
    'risk.crap',
  ],
  workspace_root: sourceRoot,
  output_root: path.join(sourceRoot, 'evidence'),
  parameters: {
    source_root: 'src',
    boundary: 'production',
    coverage: 'coverage.json',
    include_files: true,
    exclude: excluded.map((name) => `src/${name}`),
  },
};
const discovery = protocol.discover(request);
request.parameters.subjects = discovery.subjects;
request.parameters.receipt = {
  schema: 'typescript-original-coverage-receipt/v1',
  request: protocol.binding(request),
  sources: discovery.sources.map(({ path, sha256 }) => ({ path, sha256 })),
  coverage_sha256: sha(fs.readFileSync(path.join(sourceRoot, 'coverage.json'))),
  coverage_root: app,
  toolchain: {
    typescript: '6.0.2',
    instrumenter: 'istanbul-lib-instrument@6.0.3',
    instrumentation: 'original-typescript-before-transpile/v1',
    node: process.versions.node,
  },
  inputs: Object.fromEntries(
    fs
      .readdirSync(path.join(sourceRoot, 'src'), { recursive: true })
      .filter((name) => fs.statSync(path.join(sourceRoot, 'src', name)).isFile())
      .map((name) => [`src/${name}`, sha(fs.readFileSync(path.join(sourceRoot, 'src', name)))]),
  ),
  pipeline: {
    schema: 'typescript-capture-pipeline/v1',
    files: Object.fromEntries(
      [
        'package-lock.json',
        'angular.json',
        'tsconfig.json',
        'tsconfig.app.json',
        'tsconfig.spec.json',
        'tools/probe-typescript-risk.cjs',
      ].map((name) => [name, sha(read(path.join(sourceRoot, name)))]),
    ),
    tools: Object.fromEntries(
      [
        '@angular/build',
        '@angular/compiler-cli',
        'vitest',
        'jsdom',
        'istanbul-lib-instrument',
        'typescript',
      ].map((name) => [name, require(`${name}/package.json`).version]),
    ),
  },
};
write('collector-bundle.json', {
  request,
  series: protocol.series(request, request.parameters.receipt),
});
write('provenance.json', {
  status: 'local-probe-only',
  node: process.versions.node,
  plugin: JSON.parse(read(path.join(plugin, 'package.json'))).version,
  pluginFiles: Object.fromEntries(
    ['measure.cjs', 'protocol.cjs', 'strict-json.cjs', 'cli.cjs', 'npm-shrinkwrap.json'].map(
      (name) => [name, sha(read(path.join(plugin, name)))],
    ),
  ),
  config: Object.fromEntries(
    [
      'package-lock.json',
      'angular.json',
      'tsconfig.json',
      'tsconfig.spec.json',
      'tools/probe-typescript-risk.cjs',
    ].map((name) => [name, sha(read(path.join(root, name)))]),
  ),
  excludedTests: excluded.map((name) => `src/${name}`),
  merge: 'maximum per native counter; covered/uncovered only, not summed execution counts',
  limitations: [
    'No signed host receipt',
    'No complete file/template coverage certification',
    'Does not authorize a production measurement series',
  ],
});
console.log(
  JSON.stringify(
    result.map((file) => ({
      path: file.source.path,
      functions: file.functions.map((fn) => ({ name: fn.name, values: fn.values })),
    })),
    null,
    2,
  ),
);
