import assert from 'node:assert/strict';
import test from 'node:test';
import { captureCargo, captureNextest, compareInventories, parseLibtestList } from './ci-rust-test-inventory.mjs';
const context = { commit: 'fixture', rustc: 'fixture-rustc', host: 'linux', target: 'linux', profile: 'test', features: [], defaultFeatures: true };
const messages = [
  { reason: 'compiler-artifact', package_id: 'fixture#0.1.0', profile: { test: true }, target: { kind: ['lib'], name: 'fixture' }, executable: '/fixture/test' },
  { reason: 'compiler-artifact', package_id: 'fixture#0.1.0', profile: { test: true }, target: { kind: ['test'], name: 'architecture' }, executable: '/fixture/architecture' },
  { reason: 'build-finished', success: true },
].map(x => JSON.stringify(x)).join('\n');
function cargo() {
  return captureCargo(messages, context, (binary, ignored) => {
    if (binary.endsWith('architecture')) return ignored ? '0 tests, 0 benchmarks\n' : 'boundaries: test\n\n1 test, 0 benchmarks\n';
    return ignored ? 'manual_model_probe: test\n\n1 test, 0 benchmarks\n' : 'normal: test\nmanual_model_probe: test\n\n2 tests, 0 benchmarks\n';
  });
}
function nextest() {
  return captureNextest({ 'test-count': 3, 'rust-suites': {
    fixture: { 'package-id': 'fixture#0.1.0', kind: 'lib', 'binary-name': 'fixture', status: 'listed', testcases: {
      normal: { ignored: false, 'filter-match': { status: 'matches' } },
      manual_model_probe: { ignored: true, 'filter-match': { status: 'mismatch', reason: 'ignored' } },
    } },
    architecture: { 'package-id': 'fixture#0.1.0', kind: 'test', 'binary-name': 'architecture', status: 'listed',
      testcases: { boundaries: { ignored: false, 'filter-match': { status: 'matches' } } } },
  } }, context);
}
test('listing preserves ignored and integration identities without running tests', () => {
  assert.equal(compareInventories(cargo(), nextest()).ok, true);
  assert.throws(() => captureCargo(messages.replace('"success":true', '"success":false'), context, () => ''), /successful Cargo/);
  assert.throws(() => parseLibtestList('a: test\n2 tests, 0 benchmarks'), /count mismatch/);
  assert.throws(() => parseLibtestList('a: benchmark\n'), /unsupported/);
  assert.throws(() => parseLibtestList(''), /summary/);
});
test('parity rejects missing binaries, extra tests, ignored changes and changed context', () => {
  const candidate = nextest();
  candidate.binaries.pop();
  candidate.binaries[0].tests.push({ name: 'extra', ignored: false, selected: true });
  candidate.binaries[0].tests.find(x => x.ignored).ignored = false;
  candidate.context.features.push('tokio-console');
  const result = compareInventories(cargo(), candidate);
  assert.equal(result.ok, false);
  for (const pattern of [/missing binary/, /missing test/, /extra test/, /ignored policy/, /context changed/]) {
    assert(result.errors.some(error => pattern.test(error)), String(pattern));
  }
});
test('every nonignored test must be assigned exactly once across all shards', () => {
  const first = nextest(); const second = nextest();
  first.binaries[1].tests[0].selected = false;
  second.binaries[0].tests.find(x => x.name === 'normal').selected = false;
  assert.equal(compareInventories(cargo(), nextest(), [first, second]).ok, true);
  second.binaries[1].tests[0].selected = false;
  assert.equal(compareInventories(cargo(), nextest(), [first, second]).ok, false);
  second.binaries[0].tests.find(x => x.name === 'normal').selected = true;
  second.binaries[0].tests.find(x => x.ignored).selected = true;
  const result = compareInventories(cargo(), nextest(), [first, second]);
  assert(result.errors.some(x => x.includes('must be 0')));
  assert(result.errors.some(x => x.includes('must be 1')));
});
test('empty, skipped, duplicate and malformed inventories fail closed', () => {
  assert.throws(() => captureNextest({ 'test-count': 0, 'rust-suites': {} }, context), /no nextest/);
  assert.throws(() => captureNextest({ 'test-count': 0, 'rust-suites': { empty: { status: 'skipped' } } }, context), /skipped/);
  const duplicate = nextest(); duplicate.binaries.push(duplicate.binaries[0]);
  assert.throws(() => compareInventories(cargo(), duplicate), /duplicate binary/);
  const candidate = nextest(); candidate.binaries[0].tests.find(x => x.ignored).selected = true;
  assert.equal(compareInventories(cargo(), candidate).ok, false);
});
