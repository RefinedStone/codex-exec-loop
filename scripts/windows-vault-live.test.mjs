import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { STEPS, canary, holdForeignGateway, operatorResults, probeWindowsGateway, readPowerShellText, scanProductFiles, validateConfig, validateEnvironment } from './windows-vault-live.mjs';

const runId = 'a'.repeat(32);
const config = () => ({
  expectedRevision: 'b'.repeat(40),
  expectedUserSid: 'S-1-5-21-111-222-333-1001',
  acknowledgement: 'DISPOSABLE_WINDOWS_OS_ACCOUNT_AND_SANDBOX_ISSUE',
  sandboxIssueUrl: 'https://github.com/fixture-owner/fixture-repo/issues/7',
  workspace: 'C:\\fixture\\workspace',
  gatewayExecutable: 'C:\\fixture\\source\\akra-gateway.exe',
  electronExecutable: 'C:\\fixture\\source\\electron.exe',
});
async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'akra-vault-live-contract-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  for (const directory of ['akra-home', 'electron-user-data', 'captures']) await fs.mkdir(path.join(root, directory));
  return root;
}

test('manual launch requires an exact OS identity, sandbox URL, revision, and acknowledgement', () => {
  assert.deepEqual(validateConfig(config()), config());
  for (const update of [
    { expectedUserSid: '' }, { expectedRevision: 'main' }, { acknowledgement: 'yes' },
    { sandboxIssueUrl: 'https://github.com/fixture-owner/fixture-repo/issues/7?token=x' },
    { workspace: 'C:\\fixture\\..\\operator' }, { workspace: '\\\\host\\share\\repo' },
    { gatewayExecutable: 'helper.cmd' }, { approved: true },
  ]) assert.throws(() => validateConfig({ ...config(), ...update }));
});

test('automation and profile/token overrides are refused instead of being cleared', () => {
  assert.doesNotThrow(() => validateEnvironment({ PATH: 'fixture' }));
  for (const name of ['CI', 'ELECTRON_RENDERER_URL', 'ELECTRON_RUN_AS_NODE', 'AKRA_DESKTOP_HIDDEN',
    'AKRA_GATEWAY_ENDPOINT', 'AKRA_GATEWAY_BIN', 'AKRA_HOME', 'AKRA_DESKTOP_USER_DATA',
    'NODE_OPTIONS', 'CODEX_HOME', 'GH_TOKEN', 'GITHUB_TOKEN', 'OPENAI_API_KEY', 'PWDEBUG',
    'gh_token', 'Node_Options', 'playwright_test', 'pwDebug', 'akra_home', 'ci']) {
    const environment = { [name]: 'synthetic' };
    assert.throws(() => validateEnvironment(environment), name);
    assert.deepEqual(environment, { [name]: 'synthetic' });
  }
});

test('manual evidence remains operator-reported and incomplete unless explicitly recorded', () => {
  const results = operatorResults({ approve_view: { outcome: 'pass', reason: 'none' } });
  assert.equal(Object.keys(results).length, STEPS.length);
  assert.deepEqual(results.approve_view, { source: 'operator_reported', outcome: 'pass', reason: 'none' });
  assert.equal(results.cancel_view.outcome, 'not_run');
  assert.throws(() => operatorResults({ approve_view: { outcome: 'pass', reason: 'none', plaintext: canary(runId) } }));
  assert.throws(() => operatorResults({ unknown_step: { outcome: 'pass', reason: 'none' } }));
  assert.throws(() => operatorResults({ approve_view: { outcome: 'pass', reason: 'operator_not_available' } }));
});

test('owned product file scan finds UTF-8 and UTF-16 canaries across read chunk boundaries', async (t) => {
  const root = await fixture(t);
  await fs.writeFile(path.join(root, 'captures', 'utf8.log'),
    Buffer.concat([Buffer.alloc(65520, 32), Buffer.from(canary(runId))]));
  await fs.mkdir(path.join(root, 'electron-user-data', 'nested'));
  await fs.writeFile(path.join(root, 'electron-user-data', 'nested', 'utf16.bin'),
    Buffer.concat([Buffer.alloc(65530, 32), Buffer.from(canary(runId), 'utf16le')]));
  const report = await scanProductFiles(root, runId);
  assert.equal(report.result, 'canary_found');
  assert.equal(report.findings.length, 2);
  assert.equal(report.skipped.length, 0);
  assert.ok(!JSON.stringify(report).includes(canary(runId)));
});

test('scan budgets and hardlinks are explicit gaps, never a clean observation', async (t) => {
  const root = await fixture(t);
  await fs.writeFile(path.join(root, 'akra-home', 'large.db'), Buffer.alloc(64, 1));
  const budget = await scanProductFiles(root, runId, { maximumFileBytes: 32 });
  assert.equal(budget.result, 'incomplete');
  assert.equal(budget.skipped[0].reason, 'scan_budget');
  const original = path.join(root, 'akra-home', 'original.log');
  await fs.writeFile(original, canary(runId));
  await fs.link(original, path.join(root, 'captures', 'linked.log'));
  const linked = await scanProductFiles(root, runId);
  assert.equal(linked.result, 'incomplete');
  assert.equal(linked.skipped.filter((item) => item.reason === 'redirected_or_special_entry').length, 2);
});

test('a Gateway left by an earlier fixture holds the next launch instead of being followed', () => {
  const oldRoot = 'C:\\Users\\fixture\\akra-vault-live-' + 'c'.repeat(32);
  const newRoot = 'C:\\Users\\fixture\\akra-vault-live-' + 'd'.repeat(32);
  const manifest = (root) => ({ root, config: config() });
  const asked = [];
  const oldDaemon = (sid, akraHome) => {
    asked.push([sid, akraHome]);
    return { present: true, executable: oldRoot + '\\helper\\akra-gateway.exe', homeLocked: false };
  };
  assert.throws(() => holdForeignGateway(manifest(newRoot), oldDaemon), /이미 실행 중/);
  assert.deepEqual(asked, [[config().expectedUserSid, newRoot + '\\akra-home']]);
  const own = { present: true, executable: oldRoot + '\\HELPER\\akra-gateway.exe', homeLocked: true };
  assert.doesNotThrow(() => holdForeignGateway(manifest(oldRoot), () => own));
  for (const found of [{}, { ...own, homeLocked: false }, { ...own, executable: null },
    { present: true, executable: 'C:\\Program Files\\Akra\\akra-gateway.exe', homeLocked: true }]) {
    assert.throws(() => holdForeignGateway(manifest(oldRoot), () => found), /이미 실행 중/);
  }
  assert.throws(() => holdForeignGateway(manifest(newRoot), () => { throw new Error('ERROR_PIPE_BUSY'); }), /확인할 수 없습니다/);
  assert.doesNotThrow(() => holdForeignGateway(manifest(newRoot), () => ({ present: false })));
});

test('Windows Gateway probe reports an unused pipe as absent', { skip: process.platform !== 'win32' }, async (t) => {
  const root = await fixture(t);
  assert.deepEqual(probeWindowsGateway('S-1-5-21-0-0-0-0', path.join(root, 'akra-home')), { present: false });
});

test('Windows PowerShell text preserves non-ASCII paths independently of the OEM code page', { skip: process.platform !== 'win32' }, () => {
  const output = readPowerShellText("[PSCustomObject]@{path=('C:\\Users\\' + [char]0xD55C + [char]0xAE00)} | ConvertTo-Json -Compress");
  assert.equal(JSON.parse(output).path, 'C:\\Users\\한글');
});

test('Windows dry-run refuses another OS user before creating or launching a fixture', { skip: process.platform !== 'win32' }, async (t) => {
  const { spawnSync } = await import('node:child_process');
  const root = await fixture(t);
  const input = path.join(root, 'config.json');
  await fs.writeFile(input, JSON.stringify({ ...config(), expectedUserSid: 'S-1-5-21-0-0-0-0' }));
  const script = new URL('./windows-vault-live.mjs', import.meta.url);
  const { fileURLToPath } = await import('node:url');
  const result = spawnSync(process.execPath, [fileURLToPath(script), '--dry-run', input], { encoding: 'utf8', windowsHide: true });
  assert.equal(result.status, 1);
  assert.ok(result.stderr.includes('현재 OS 사용자와 명시한 폐기 사용자 SID가 다릅니다.'));
  assert.equal(result.stdout, '');
  assert.deepEqual((await fs.readdir(root)).sort(), ['akra-home', 'captures', 'config.json', 'electron-user-data']);
});

test('Windows staging creates only a private byte-identical copy and refuses reuse', { skip: process.platform !== 'win32' }, async (t) => {
  const { execFileSync, spawnSync } = await import('node:child_process');
  const { randomUUID, createHash } = await import('node:crypto');
  const { fileURLToPath } = await import('node:url');
  const powershell = path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
  const identity = JSON.parse(readPowerShellText(
    '[PSCustomObject]@{sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; interactive=[Environment]::UserInteractive; profile=[Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)} | ConvertTo-Json -Compress'));
  if (!identity.interactive) { t.skip('file staging requires an interactive OS profile; no keyring or UI was called'); return; }
  const id = randomUUID().replaceAll('-', '');
  const root = path.join(identity.profile, 'akra-vault-live-' + id);
  // Only a fresh random directory can enter cleanup. No executable is launched.
  await assert.rejects(fs.lstat(root), { code: 'ENOENT' });
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const stage = fileURLToPath(new URL('./windows-vault-live/stage.ps1', import.meta.url));
  const nativeSource = path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'whoami.exe');
  const args = ['-NoProfile', '-NonInteractive', '-File', stage, '-ExpectedSid', identity.sid,
    '-GatewayExecutable', nativeSource, '-RunId', id];
  const result = spawnSync(powershell, args, { encoding: 'utf8', windowsHide: true });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).root, root);
  const hash = (data) => createHash('sha256').update(data).digest('hex');
  const expected = hash(await fs.readFile(nativeSource));
  const copied = path.join(root, 'helper', 'akra-gateway.exe');
  assert.equal(hash(await fs.readFile(copied)), expected);
  const quote = (value) => "'" + value.replaceAll("'", "''") + "'";
  for (const relative of ['', 'helper', 'akra-home', 'electron-user-data', 'captures', 'helper/akra-gateway.exe']) {
    const entry = quote(path.join(root, relative));
    const command = '$acl=if([IO.File]::Exists(' + entry + ')){[IO.File]::GetAccessControl(' + entry + ')}else{[IO.Directory]::GetAccessControl(' + entry + ')}' +
      '; [PSCustomObject]@{protected=$acl.AreAccessRulesProtected;owner=$acl.GetOwner([Security.Principal.SecurityIdentifier]).Value;other=@($acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]) | Where-Object { $_.IdentityReference.Value -ne ' +
      quote(identity.sid) + ' }).Count} | ConvertTo-Json -Compress';
    const acl = JSON.parse(execFileSync(powershell, ['-NoProfile', '-NonInteractive', '-Command', command], { encoding: 'utf8', windowsHide: true }));
    assert.equal(acl.protected, true);
    assert.equal(acl.owner, identity.sid);
    assert.equal(acl.other, 0);
  }
  const reused = spawnSync(powershell, args, { encoding: 'utf8', windowsHide: true });
  assert.notEqual(reused.status, 0);
  assert.ok(reused.stderr.includes('Never reuse or repair an existing fixture root.'));
  assert.equal(hash(await fs.readFile(copied)), expected);
});

test('missing product roots and redirected profile directories are scan gaps', async (t) => {
  const root = await fixture(t);
  await fs.rm(path.join(root, 'captures'), { recursive: true });
  await fs.rm(path.join(root, 'electron-user-data'), { recursive: true });
  const outside = path.join(root, 'outside-product-roots');
  await fs.mkdir(outside);
  await fs.writeFile(path.join(outside, 'not-owned-by-product.txt'), canary(runId));
  await fs.symlink(outside, path.join(root, 'electron-user-data'), 'junction');
  const report = await scanProductFiles(root, runId);
  assert.equal(report.result, 'incomplete');
  assert.equal(report.findings.length, 0);
  assert.ok(report.skipped.some((entry) => entry.path === 'captures' && entry.reason === 'unreadable_directory'));
  assert.ok(report.skipped.some((entry) => entry.path === 'electron-user-data' && entry.reason === 'redirected_or_special_entry'));
});

test('a changing product directory cannot become a clean file observation', async (t) => {
  const root = await fixture(t);
  const directory = path.join(root, 'akra-home');
  for (let i = 0; i < 64; i++) await fs.writeFile(path.join(directory, 'file-' + i), Buffer.alloc(8192));
  let scanning = true;
  const pending = scanProductFiles(root, runId).finally(() => { scanning = false; });
  const changes = (async () => {
    let index = 0;
    while (scanning) {
      const file = path.join(directory, 'changing-' + index++);
      await fs.writeFile(file, 'synthetic directory change');
      await fs.unlink(file);
      await new Promise(setImmediate);
    }
  })();
  const [report] = await Promise.all([pending, changes]);
  assert.equal(report.result, 'incomplete');
  assert.ok(report.skipped.some((entry) => entry.path === 'akra-home' && entry.reason === 'changed_directory'));
});
