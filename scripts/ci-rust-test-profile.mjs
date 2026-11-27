#!/usr/bin/env node
// Opt-in diagnosis of already-built libtest executables; never the default CI runner.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { parseLibtestList } from './ci-rust-test-inventory.mjs';

function ensure(condition, message) {
  if (!condition) throw new Error(message);
}

export function profileLibtest(binary, requested, run, now = () => performance.now()) {
  const list = ignored => {
    const result = run(binary, ['--list', '--format=pretty', ...(ignored ? ['--ignored'] : [])]);
    ensure(result.status === 0 && !result.signal && !result.error,
      'libtest inventory failed: ' + (result.error?.message ?? result.stderr ?? result.status));
    return parseLibtestList(result.stdout);
  };
  const inventory = list(false);
  const ignored = new Set(list(true));
  ensure([...ignored].every(name => inventory.includes(name)), 'ignored inventory differs from full inventory');
  ensure(new Set(requested).size === requested.length, 'duplicate requested test');
  for (const name of requested) {
    ensure(inventory.includes(name), 'unknown test: ' + name);
    ensure(!ignored.has(name), 'ignored tests are not profiling targets: ' + name);
  }
  const selected = requested.length ? [...requested] : inventory.filter(name => !ignored.has(name));
  ensure(selected.length > 0, 'no active tests selected');
  const report = {
    schemaVersion: 1,
    measurement: 'one test per process, sequential; includes process startup and teardown',
    equivalentToDefaultSuite: false,
    binary,
    inventoryCount: inventory.length,
    ignored: [...ignored].sort(),
    selection: requested.length ? 'explicit' : 'all-active',
    selected,
    results: [],
    complete: false,
  };
  for (const name of selected) {
    const startedAt = new Date().toISOString();
    const begin = now();
    const result = run(binary, ['--exact', name, '--test-threads=1', '--format=pretty', '--color=never']);
    const elapsedMs = now() - begin;
    const summary = result.stdout?.match(/^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; \d+ filtered out; finished in ([\d.]+)s$/m);
    const passed = result.status === 0 && !result.signal && !result.error && Boolean(summary);
    report.results.push({
      name, startedAt, elapsedMs,
      libtestSeconds: summary ? Number(summary[1]) : null,
      passed, exitCode: result.status, signal: result.signal ?? null,
      error: result.error?.message ?? null,
      stdout: result.stdout ?? '', stderr: result.stderr ?? '',
    });
    if (!passed) return report;
  }
  report.complete = true;
  return report;
}

export function main(args) {
  const [binaryArgument, output, ...requested] = args;
  ensure(binaryArgument && output,
    'usage: ci-rust-test-profile.mjs <local-test-executable> <output.json> [exact-test-name ...]');
  const binary = fs.realpathSync(binaryArgument);
  ensure(fs.statSync(binary).isFile(), 'test executable must be a local file');
  const outputPath = fs.existsSync(output) ? fs.realpathSync(output) : path.resolve(output);
  ensure(outputPath !== binary, 'output must not overwrite the test executable');
  const metadata = {
    binary, binarySha256: createHash('sha256').update(fs.readFileSync(binary)).digest('hex'),
    platform: process.platform, architecture: process.arch, workingDirectory: process.cwd(),
  };
  // A cancelled or failed inventory read must not leave a prior successful receipt.
  fs.writeFileSync(output, JSON.stringify({ schemaVersion: 1, complete: false, ...metadata }) + '\n');
  const run = (executable, arguments_) => spawnSync(executable, arguments_, {
    encoding: 'utf8', shell: false, maxBuffer: 16 * 1024 * 1024,
    // Profiling preserves the executable's own timeouts. Bound this diagnostic
    // command with the caller's normal job/process supervisor when needed.
  });
  let report;
  try {
    report = { ...profileLibtest(binary, requested, run), ...metadata };
  } catch (error) {
    fs.writeFileSync(output, JSON.stringify({
      schemaVersion: 1, complete: false, error: error.message, ...metadata,
    }, null, 2) + '\n');
    throw error;
  }
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  process.stdout.write(JSON.stringify({
    output, complete: report.complete, selected: report.selected.length,
    measured: report.results.length,
    slowest: [...report.results].sort((a, b) => b.elapsedMs - a.elapsedMs)
      .slice(0, 20).map(({ name, elapsedMs, passed }) => ({ name, elapsedMs, passed })),
  }, null, 2) + '\n');
  if (!report.complete) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(process.argv.slice(2)); }
  catch (error) { process.stderr.write(error.message + '\n'); process.exitCode = 1; }
}
