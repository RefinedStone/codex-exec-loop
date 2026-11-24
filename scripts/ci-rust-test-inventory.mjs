#!/usr/bin/env node
// Opt-in inventory comparison; production CI continues to use cargo test.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

function ensure(ok, message) { if (!ok) throw new Error(message); }
function object(value, label) {
  ensure(value && typeof value === 'object' && !Array.isArray(value), label + ' must be an object');
  return value;
}
function text(value, label) {
  ensure(typeof value === 'string' && value.length > 0, label + ' must be nonempty');
  return value;
}
function identity(packageId, kind, name) {
  return JSON.stringify([text(packageId, 'package id'), text(kind, 'kind'), text(name, 'name')]);
}
export function validateContext(value) {
  object(value, 'context');
  const result = {};
  for (const key of ['commit', 'rustc', 'host', 'target', 'profile']) result[key] = text(value[key], key);
  ensure(Array.isArray(value.features) && value.features.every(x => typeof x === 'string'), 'features must be strings');
  result.features = [...new Set(value.features)].sort();
  ensure(typeof value.defaultFeatures === 'boolean', 'defaultFeatures must be boolean');
  result.defaultFeatures = value.defaultFeatures;
  return result;
}
export function parseLibtestList(output) {
  const names = [];
  let summary;
  for (const line of output.split(/\r?\n/).map(x => x.trim()).filter(Boolean)) {
    const test = line.match(/^(.*): test$/);
    const count = line.match(/^(\d+) tests?, (\d+) benchmarks?$/);
    if (test) names.push(test[1]);
    else if (count) {
      ensure(!summary, 'duplicate libtest summary');
      summary = [Number(count[1]), Number(count[2])];
    } else throw new Error('unsupported libtest list line: ' + line);
  }
  ensure(summary && summary[0] === names.length && summary[1] === 0, 'missing libtest summary, count mismatch or unsupported benchmark');
  ensure(new Set(names).size === names.length, 'duplicate libtest name');
  return names.sort();
}
export function captureCargo(messages, context, listBinary) {
  const records = messages.split(/\r?\n/).filter(x => x.trim()).map(x => JSON.parse(x));
  ensure(records.some(x => x.reason === 'build-finished' && x.success === true), 'missing successful Cargo build');
  ensure(!records.some(x => x.reason === 'build-finished' && x.success === false), 'Cargo build failed');
  const binaries = [];
  const ids = new Set();
  for (const record of records) {
    if (record.reason !== 'compiler-artifact' || record.profile?.test !== true || !record.executable) continue;
    const kind = record.target?.kind;
    ensure(Array.isArray(kind) && kind.length === 1 && ['lib', 'bin', 'test'].includes(kind[0]), 'unsupported Cargo target kind');
    const id = identity(record.package_id, kind[0], record.target.name);
    ensure(!ids.has(id), 'duplicate Cargo binary');
    ids.add(id);
    const names = parseLibtestList(listBinary(record.executable, false));
    const ignored = new Set(parseLibtestList(listBinary(record.executable, true)));
    ensure([...ignored].every(name => names.includes(name)), 'ignored test absent from full list');
    binaries.push({ id, tests: names.map(name => ({ name, ignored: ignored.has(name), selected: !ignored.has(name) })) });
  }
  ensure(binaries.length > 0, 'no Cargo test binaries');
  return { schemaVersion: 1, runner: 'cargo', context: validateContext(context), binaries };
}
export function captureNextest(input, context) {
  object(input, 'nextest list');
  const binaries = [];
  const ids = new Set();
  let count = 0;
  for (const suite of Object.values(object(input['rust-suites'], 'rust-suites'))) {
    object(suite, 'suite');
    ensure(suite.status === undefined || suite.status === 'listed', 'skipped nextest suite: full inventory required');
    ensure(['lib', 'bin', 'test'].includes(suite.kind), 'unsupported nextest target kind');
    const id = identity(suite['package-id'], suite.kind, suite['binary-name']);
    ensure(!ids.has(id), 'duplicate nextest binary');
    ids.add(id);
    const tests = Object.entries(object(suite.testcases, 'testcases')).map(([name, test]) => {
      ensure(test.kind == null || test.kind === 'test', 'unsupported nextest test kind');
      ensure(typeof test.ignored === 'boolean', 'missing ignored flag');
      const status = test['filter-match']?.status;
      ensure(['matches', 'mismatch'].includes(status), 'missing or unknown filter status');
      return { name, ignored: test.ignored, selected: status === 'matches' };
    }).sort((a, b) => a.name.localeCompare(b.name));
    count += tests.length;
    binaries.push({ id, tests });
  }
  ensure(binaries.length > 0, 'no nextest binaries');
  ensure(Number.isInteger(input['test-count']) && input['test-count'] === count, 'nextest test-count mismatch');
  return { schemaVersion: 1, runner: 'nextest', context: validateContext(context), binaries };
}
function flatten(inventory) {
  ensure(inventory.schemaVersion === 1, 'unsupported inventory schema');
  validateContext(inventory.context);
  ensure(Array.isArray(inventory.binaries) && inventory.binaries.length > 0, 'missing binaries');
  const entries = new Map();
  const binaries = new Set();
  for (const binary of inventory.binaries) {
    text(binary.id, 'binary id');
    ensure(!binaries.has(binary.id), 'duplicate binary');
    binaries.add(binary.id);
    ensure(Array.isArray(binary.tests), 'missing tests');
    for (const test of binary.tests) {
      const key = JSON.stringify([binary.id, text(test.name, 'test name')]);
      ensure(typeof test.ignored === 'boolean' && typeof test.selected === 'boolean', 'invalid ignored/selected flags');
      ensure(!entries.has(key), 'duplicate test');
      entries.set(key, test);
    }
  }
  return { entries, binaries };
}
export function compareInventories(baseline, candidate, shards = []) {
  ensure(baseline.runner === 'cargo' && candidate.runner === 'nextest', 'expected Cargo baseline and nextest candidate');
  const expected = flatten(baseline);
  const errors = [];
  const context = JSON.stringify(validateContext(baseline.context));
  function checkInventory(inventory, label) {
    const actual = flatten(inventory);
    if (JSON.stringify(validateContext(inventory.context)) !== context) errors.push(label + ': build context changed');
    for (const id of expected.binaries) if (!actual.binaries.has(id)) errors.push(label + ': missing binary ' + id);
    for (const id of actual.binaries) if (!expected.binaries.has(id)) errors.push(label + ': extra binary ' + id);
    for (const [key, test] of expected.entries) {
      const other = actual.entries.get(key);
      if (!other) errors.push(label + ': missing test ' + key);
      else if (other.ignored !== test.ignored) errors.push(label + ': ignored policy changed ' + key);
    }
    for (const key of actual.entries.keys()) if (!expected.entries.has(key)) errors.push(label + ': extra test ' + key);
    return actual.entries;
  }
  for (const [key, test] of checkInventory(candidate, 'nextest')) {
    const original = expected.entries.get(key);
    if (original && test.selected !== !original.ignored) errors.push('nextest: default selection changed ' + key);
  }
  if (shards.length > 0) {
    ensure(shards.length >= 2, 'provide all shards, at least two');
    const assignments = new Map();
    shards.forEach((shard, index) => {
      ensure(shard.runner === 'nextest', 'shard must use nextest');
      for (const [key, test] of checkInventory(shard, 'shard ' + (index + 1))) {
        if (test.selected) assignments.set(key, (assignments.get(key) ?? 0) + 1);
      }
    });
    for (const [key, test] of expected.entries) {
      const wanted = test.ignored ? 0 : 1;
      if ((assignments.get(key) ?? 0) !== wanted) errors.push('shard coverage must be ' + wanted + ': ' + key);
    }
  }
  return { ok: errors.length === 0, binaries: expected.binaries.size, tests: expected.entries.size,
    ignored: [...expected.entries.values()].filter(x => x.ignored).length, shards: shards.length, errors };
}
function readJson(filename) { return JSON.parse(fs.readFileSync(filename, 'utf8')); }
function writeJson(filename, value) { fs.writeFileSync(filename, JSON.stringify(value, null, 2) + '\n'); }
export function main(args) {
  const [command, ...rest] = args;
  if (command === 'capture-cargo' && rest.length === 3) {
    const inventory = captureCargo(fs.readFileSync(rest[0], 'utf8'), readJson(rest[1]), (binary, ignored) => {
      const result = spawnSync(binary, ['--list', '--format', 'pretty', ...(ignored ? ['--ignored'] : [])], {
        encoding: 'utf8', timeout: 30_000, maxBuffer: 16 * 1024 * 1024, shell: false,
      });
      if (result.error) throw result.error;
      ensure(result.status === 0, 'test binary listing failed: ' + binary);
      return result.stdout;
    });
    writeJson(rest[2], inventory);
  } else if (command === 'capture-nextest' && rest.length === 3) {
    writeJson(rest[2], captureNextest(readJson(rest[0]), readJson(rest[1])));
  } else if (command === 'compare' && rest.length >= 2) {
    const result = compareInventories(readJson(rest[0]), readJson(rest[1]), rest.slice(2).map(readJson));
    process.stdout.write(JSON.stringify(result, null, 2) + '\n');
    if (!result.ok) process.exitCode = 1;
  } else throw new Error('usage: ci-rust-test-inventory.mjs capture-cargo <cargo.jsonl> <context.json> <out.json> | capture-nextest <list.json> <context.json> <out.json> | compare <cargo.json> <nextest.json> [shard.json ...]');
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(process.argv.slice(2)); }
  catch (error) { process.stderr.write(error.message + '\n'); process.exitCode = 1; }
}
