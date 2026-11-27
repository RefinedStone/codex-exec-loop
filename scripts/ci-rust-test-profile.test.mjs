import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { profileLibtest } from './ci-rust-test-profile.mjs';

function fixture(result = {}) {
  const calls = [];
  return {
    calls,
    run(binary, args) {
      calls.push({ binary, args });
      if (args.includes('--list')) return {
        status: 0,
        stdout: args.includes('--ignored')
          ? 'manual_model_probe: test\n\n1 test, 0 benchmarks\n'
          : 'slow: test\nfast: test\nmanual_model_probe: test\n\n3 tests, 0 benchmarks\n',
      };
      return { status: 0, stdout: 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.01s\n', ...result };
    },
  };
}

test('profiles the active inventory once without enabling ignored model probes', () => {
  const fake = fixture();
  let time = 0;
  const report = profileLibtest('local.exe', [], fake.run, () => time += 10);
  assert.equal(report.complete, true);
  assert.deepEqual(report.selected, ['fast', 'slow']);
  assert.deepEqual(report.ignored, ['manual_model_probe']);
  assert.equal(report.equivalentToDefaultSuite, false);
  assert.deepEqual(report.results.map(row => row.elapsedMs), [10, 10]);
  assert.deepEqual(fake.calls.slice(2).map(call => call.args), [
    ['--exact', 'fast', '--test-threads=1', '--format=pretty', '--color=never'],
    ['--exact', 'slow', '--test-threads=1', '--format=pretty', '--color=never'],
  ]);
});

test('rejects missing, duplicate and ignored names before execution', () => {
  for (const names of [['absent'], ['slow', 'slow'], ['manual_model_probe']]) {
    const fake = fixture();
    assert.throws(() => profileLibtest('local.exe', names, fake.run));
    assert.equal(fake.calls.length, 2);
  }
});

test('records failure and leaves unexecuted tests incomplete', () => {
  const fake = fixture({ status: 101, stdout: 'test result: FAILED. 0 passed; 1 failed\n' });
  const report = profileLibtest('local.exe', ['slow', 'fast'], fake.run);
  assert.equal(report.complete, false);
  assert.equal(report.results.length, 1);
  assert.equal(report.results[0].passed, false);
  assert.equal(report.results[0].exitCode, 101);
  assert.equal(fake.calls.length, 3);
});

test('zero matches, signal termination and spawn errors cannot count as successful measurements', () => {
  for (const result of [
    { stdout: 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.00s\n' },
    { status: null, signal: 'SIGTERM' },
    { status: null, error: new Error('spawn failed') },
  ]) {
    const fake = fixture(result);
    const report = profileLibtest('local.exe', ['slow'], fake.run);
    assert.equal(report.complete, false);
    assert.equal(report.results[0].passed, false);
  }
});

test('inventory failures prevent execution', () => {
  const calls = [];
  assert.throws(() => profileLibtest('local.exe', [], (_, args) => {
    calls.push(args);
    return { status: 101, stdout: '', stderr: 'failed to list' };
  }), /inventory failed/);
  assert.equal(calls.length, 1);
});

test('CLI replaces stale success with an incomplete receipt when inventory fails', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'akra-test-profile-'));
  try {
    const output = path.join(directory, 'profile.json');
    fs.writeFileSync(output, JSON.stringify({ complete: true }));
    const script = fileURLToPath(new URL('./ci-rust-test-profile.mjs', import.meta.url));
    // Node is a local executable that rejects libtest's --list arguments.
    const result = spawnSync(process.execPath, [script, process.execPath, output], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    const report = JSON.parse(fs.readFileSync(output, 'utf8'));
    assert.equal(report.complete, false);
    assert.match(report.error, /inventory failed/);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
