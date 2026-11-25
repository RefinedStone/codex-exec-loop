#!/usr/bin/env node
// Manual Windows product launch only. No private-channel interception, UI injection,
// credential calls, model requests, or overrides of the product's viewing guards.
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { finished } from 'node:stream/promises';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const STAGING = path.join(repository, 'scripts', 'windows-vault-live', 'stage.ps1');
const ACK = 'DISPOSABLE_WINDOWS_OS_ACCOUNT_AND_SANDBOX_ISSUE';
const MAX_CAPTURE = 8 * 1024 * 1024;
export const STEPS = Object.freeze([
  'cancel_view', 'approve_view', 'restart_restore', 'external_edit_hold',
  'separate_reconnect_approval', 'window_close', 'window_reload',
  'project_issue_switch', 'os_lock', 'five_minute_expiry',
]);
const OUTCOMES = new Set(['not_run', 'pass', 'fail', 'blocked']);
const REASONS = new Set(['none', 'operator_not_available', 'disposable_profile_missing',
  'sandbox_issue_missing', 'native_confirmation_unavailable', 'product_hold', 'unexpected_result']);
const required = ['expectedRevision', 'expectedUserSid', 'acknowledgement', 'sandboxIssueUrl',
  'workspace', 'gatewayExecutable', 'electronExecutable'];

function fail(message) { throw new Error(message); }
function plain(value) { return value !== null && typeof value === 'object' && !Array.isArray(value); }
function exact(value, fields) {
  return plain(value) && Object.keys(value).every((key) => fields.includes(key));
}
function drivePath(value) {
  return typeof value === 'string' && /^[A-Za-z]:[\\/]/.test(value) &&
    !/[\u0000-\u001f]/.test(value) &&
    !value.split(/[\\/]/).some((part) => part === '.' || part === '..');
}

export function validateConfig(value) {
  if (!exact(value, required) || required.some((key) => typeof value[key] !== 'string')) {
    fail('설정 필드가 누락되었거나 허용하지 않는 필드가 있습니다.');
  }
  if (!/^[a-f0-9]{40}$/.test(value.expectedRevision)) fail('전체 Git SHA 40자가 필요합니다.');
  if (!/^S-1-5-21-(?:\d+-){3}\d+$/.test(value.expectedUserSid)) fail('폐기 Windows 사용자 SID가 필요합니다.');
  if (value.acknowledgement !== ACK) fail('폐기 OS 사용자와 sandbox 이슈 확인이 필요합니다.');
  if (!/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+\/issues\/[1-9]\d*$/.test(value.sandboxIssueUrl)) {
    fail('명시적인 GitHub sandbox 이슈 URL이 필요합니다.');
  }
  for (const name of ['workspace', 'gatewayExecutable', 'electronExecutable']) {
    if (!drivePath(value[name])) fail('설정 경로는 별칭 없는 Windows 드라이브 절대경로여야 합니다.');
  }
  if (!value.gatewayExecutable.toLowerCase().endsWith('.exe') ||
      !value.electronExecutable.toLowerCase().endsWith('.exe')) fail('Windows 실행 파일을 지정하세요.');
  return Object.freeze({ ...value });
}

export function validateEnvironment(environment) {
  const forbidden = ['CI', 'ELECTRON_RENDERER_URL', 'ELECTRON_RUN_AS_NODE', 'AKRA_DESKTOP_HIDDEN',
    'AKRA_GATEWAY_ENDPOINT', 'AKRA_GATEWAY_BIN', 'AKRA_DESKTOP_USER_DATA', 'AKRA_HOME',
    'NODE_OPTIONS', 'CODEX_HOME', 'GITHUB_TOKEN', 'GH_TOKEN', 'OPENAI_API_KEY', 'CODEX_API_KEY',
    'ANTHROPIC_API_KEY'];
  if (Object.entries(environment).some(([name, value]) => {
    const key = name.toUpperCase();
    return Boolean(value) && (forbidden.includes(key) || /^(PLAYWRIGHT|PWDEBUG)/.test(key));
  })) {
    fail('자동화·개발·프로필·토큰 override가 있는 환경에서는 실행하지 않습니다. 폐기 계정의 일반 터미널을 사용하세요.');
  }
}

export function operatorResults(value) {
  if (!plain(value) || Object.keys(value).some((key) => !STEPS.includes(key))) fail('알 수 없는 수동 검증 단계입니다.');
  return Object.fromEntries(STEPS.map((step) => {
    const result = value[step] ?? { outcome: 'not_run', reason: 'none' };
    if (!exact(result, ['outcome', 'reason']) || !OUTCOMES.has(result.outcome) ||
        !REASONS.has(result.reason) || (result.outcome === 'pass' && result.reason !== 'none')) {
      fail('수동 결과는 지정된 outcome/reason만 기록할 수 있습니다. 원문이나 자유 텍스트를 넣지 마세요.');
    }
    return [step, { source: 'operator_reported', outcome: result.outcome, reason: result.reason }];
  }));
}

const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
export function canary(runId) {
  if (!/^[a-f0-9]{32}$/.test(runId)) fail('검증 실행 ID가 올바르지 않습니다.');
  return 'AKRA_VAULT_LIVE_CANARY_' + runId;
}
async function hashFile(file) {
  const hash = createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}
function readJson(file) {
  const info = fs.lstatSync(file);
  if (!info.isFile() || info.isSymbolicLink() || info.size > 128 * 1024) fail('작은 일반 JSON 파일이 필요합니다.');
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}
function powershell() {
  return path.win32.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32',
    'WindowsPowerShell', 'v1.0', 'powershell.exe');
}
export function readPowerShellText(command) {
  // Windows PowerShell otherwise writes the current OEM code page (for example CP949).
  // Explicit UTF-8 preserves non-ASCII profile paths in Node's UTF-8 decoder.
  return execFileSync(powershell(), ['-NoProfile', '-NonInteractive', '-Command',
    '[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); ' + command],
  { encoding: 'utf8', windowsHide: true }).trim();
}
function currentSid() {
  return readPowerShellText('[Security.Principal.WindowsIdentity]::GetCurrent().User.Value');
}
function git(args) {
  return execFileSync('git', ['-C', repository, ...args], { encoding: 'utf8', windowsHide: true }).trim();
}
function assertCurrentUser(expected) {
  if (process.platform !== 'win32') fail('이 수동 실행기는 Windows에서만 사용할 수 있습니다.');
  if (currentSid() !== expected) fail('현재 OS 사용자와 명시한 폐기 사용자 SID가 다릅니다. 파일과 credential을 건드리지 않았습니다.');
}
function regular(file) {
  const info = fs.lstatSync(file);
  if (!info.isFile() || info.isSymbolicLink()) fail('실제 일반 파일만 사용할 수 있습니다.');
}
async function preflight(config) {
  assertCurrentUser(config.expectedUserSid);
  validateEnvironment(process.env);
  if (git(['rev-parse', 'HEAD']) !== config.expectedRevision) fail('현재 checkout SHA가 설정과 다릅니다.');
  if (git(['status', '--porcelain', '--untracked-files=normal'])) fail('변경 없는 checkout에서만 제품 빌드를 실행하세요.');
  if (!fs.statSync(config.workspace).isDirectory()) fail('sandbox checkout이 없습니다.');
  if (fs.lstatSync(config.workspace).isSymbolicLink()) fail('sandbox checkout의 junction/symlink 별칭은 사용할 수 없습니다.');
  const files = [config.gatewayExecutable, config.electronExecutable,
    path.join(repository, 'desktop', 'out', 'main', 'index.js'),
    path.join(repository, 'desktop', 'out', 'preload', 'vaultRestore.js'),
    path.join(repository, 'desktop', 'out', 'renderer', 'vault-restore.html')];
  const hashes = [];
  for (const file of files) { regular(file); hashes.push(await hashFile(file)); }
  return { files, hashes };
}

export async function scanProductFiles(root, runId, limits = {}) {
  const maximumFileBytes = limits.maximumFileBytes ?? 64 * 1024 * 1024;
  const maximumTotalBytes = limits.maximumTotalBytes ?? 512 * 1024 * 1024;
  const maximumFiles = limits.maximumFiles ?? 10000;
  const needles = [Buffer.from(canary(runId), 'utf8'), Buffer.from(canary(runId), 'utf16le')];
  const overlap = Math.max(...needles.map((needle) => needle.length)) - 1;
  const findings = [];
  const skipped = [];
  let bytes = 0;
  let files = 0;
  const gap = (file, reason) => skipped.push({ path: path.relative(root, file), reason });
  async function visit(directory) {
    let entries;
    let before;
    try {
      const info = await fsp.lstat(directory);
      if (!info.isDirectory() || info.isSymbolicLink()) return gap(directory, 'redirected_or_special_entry');
      before = info;
      entries = await fsp.readdir(directory, { withFileTypes: true });
    } catch { return gap(directory, 'unreadable_directory'); }
    for (const entry of entries) {
      const file = path.join(directory, entry.name);
      const relative = path.relative(root, file);
      let handle;
      try {
        const info = await fsp.lstat(file);
        if (info.isSymbolicLink() || (info.isFile() && info.nlink > 1) || (!info.isFile() && !info.isDirectory())) {
          gap(file, 'redirected_or_special_entry');
        } else if (info.isDirectory()) {
          await visit(file);
        } else if (info.size > maximumFileBytes || bytes + info.size > maximumTotalBytes || files >= maximumFiles) {
          gap(file, 'scan_budget');
        } else {
          handle = await fsp.open(file, 'r');
          const opened = await handle.stat();
          if (!opened.isFile() || opened.nlink > 1 || opened.dev !== info.dev || opened.ino !== info.ino) {
            gap(file, 'redirected_or_special_entry');
            continue;
          }
          files++;
          let read = 0;
          let tail = Buffer.alloc(0);
          let found = false;
          for await (const chunk of handle.createReadStream({ autoClose: false })) {
            if (read + chunk.length > maximumFileBytes || bytes + chunk.length > maximumTotalBytes) {
              gap(file, 'scan_budget');
              break;
            }
            read += chunk.length;
            bytes += chunk.length;
            const buffer = Buffer.concat([tail, chunk]);
            if (needles.some((needle) => buffer.includes(needle))) found = true;
            tail = Buffer.from(buffer.subarray(Math.max(0, buffer.length - overlap)));
          }
          if (found) findings.push(relative);
          const after = await handle.stat();
          if (after.size !== opened.size || after.mtimeMs !== opened.mtimeMs) gap(file, 'unreadable_or_changed_entry');
        }
      } catch { gap(file, 'unreadable_or_changed_entry'); }
      finally { await handle?.close(); }
    }
    try {
      const after = await fsp.lstat(directory);
      if (!after.isDirectory() || after.isSymbolicLink() || after.dev !== before.dev ||
          after.ino !== before.ino || after.mtimeMs !== before.mtimeMs) gap(directory, 'changed_directory');
    } catch { gap(directory, 'changed_directory'); }
  }
  // Only this run's product storage and captured process output are read.
  // Never enumerate the OS keyring, other profiles, or operator note/config files.
  for (const directory of ['akra-home', 'electron-user-data', 'captures']) await visit(path.join(root, directory));
  return { source: 'automated_file_scan', files, bytes, findings, skipped,
    result: findings.length ? 'canary_found' : skipped.length ? 'incomplete' : 'no_canary_observed' };
}

async function writeJson(file, value, flag = 'w') {
  await fsp.writeFile(file, JSON.stringify(value, null, 2) + '\n', { flag });
}
function ownedManifest(file) {
  const manifest = readJson(file);
  if (manifest.version !== 1 || !/^[a-f0-9]{32}$/.test(manifest.runId) ||
      path.resolve(file) !== path.join(path.resolve(manifest.root), 'run.json') ||
      path.basename(manifest.root) !== 'akra-vault-live-' + manifest.runId) fail('이 도구가 만든 run.json이 필요합니다.');
  validateConfig(manifest.config);
  assertCurrentUser(manifest.config.expectedUserSid);
  const profile = readPowerShellText('[Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)');
  if (path.dirname(fs.realpathSync.native(manifest.root)) !== fs.realpathSync.native(profile)) {
    fail('실행 폴더는 현재 OS 프로필 바로 아래의 전용 폴더여야 합니다.');
  }
  if (manifest.userSidSha256 !== digest(manifest.config.expectedUserSid)) fail('실행 프로필 기록이 일치하지 않습니다.');
  for (const name of ['', 'helper', 'akra-home', 'electron-user-data', 'captures']) {
    const directory = path.join(manifest.root, name);
    const info = fs.lstatSync(directory);
    if (!info.isDirectory() || info.isSymbolicLink()) fail('실행 폴더가 교체되거나 별칭으로 바뀌었습니다.');
  }
  return manifest;
}
async function launch(manifest) {
  validateEnvironment(process.env);
  const fresh = await preflight(manifest.config);
  if (JSON.stringify(fresh.hashes) !== JSON.stringify(manifest.artifacts.hashes)) fail('원본 빌드 파일이 실행 준비 이후 변경되었습니다.');
  const helper = path.join(manifest.root, 'helper', 'akra-gateway.exe');
  if (await hashFile(helper) !== manifest.artifacts.hashes[0]) fail('private helper 사본의 SHA-256이 원본과 다릅니다.');
  const attempt = manifest.launches.length + 1;
  const captures = ['stdout', 'stderr'].map((stream) =>
    fs.createWriteStream(path.join(manifest.root, 'captures', 'electron-' + attempt + '-' + stream + '.log'), { flags: 'wx' }));
  const startedAt = new Date().toISOString();
  const child = spawn(manifest.config.electronExecutable, [path.join(repository, 'desktop')], {
    cwd: repository, windowsHide: false, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, AKRA_HOME: path.join(manifest.root, 'akra-home'),
      AKRA_DESKTOP_USER_DATA: path.join(manifest.root, 'electron-user-data'), AKRA_GATEWAY_BIN: helper },
  });
  let truncated = false;
  const captured = [0, 0];
  [child.stdout, child.stderr].forEach((stream, index) => {
    stream.on('data', (chunk) => {
      if (captured[index] + chunk.length <= MAX_CAPTURE) captures[index].write(chunk);
      else truncated = true;
      captured[index] += chunk.length;
    });
    stream.on('end', () => captures[index].end());
  });
  console.log('실제 제품 창을 열었습니다. 승인/취소/OS 잠금은 사람이 직접 수행하세요.');
  console.log('이 실행의 합성 canary: ' + canary(manifest.runId));
  console.log('sandbox 이슈: ' + manifest.config.sandboxIssueUrl);
  const exit = await new Promise((resolve) => {
    child.once('error', () => resolve({ code: null, signal: null, launchError: true }));
    child.once('close', (code, signal) => resolve({ code, signal, launchError: false }));
  });
  captures.forEach((stream) => stream.end());
  await Promise.all(captures.map((stream) => finished(stream)));
  manifest.launches.push({ source: 'observed_process', startedAt, endedAt: new Date().toISOString(),
    ...exit, capturedBytes: captured, captureTruncated: truncated });
  await writeJson(path.join(manifest.root, 'run.json'), manifest);
  console.log('제품 프로세스 종료를 기록했습니다. 실제 UI 결과는 자동으로 통과 처리하지 않았습니다.');
}

async function cli(args) {
  if (args.length < 2 || !['--dry-run', '--launch', '--resume', '--report'].includes(args[0])) {
    fail('사용법: node scripts/windows-vault-live.mjs --dry-run|--launch config.json / --resume run.json / --report run.json operator-results.json');
  }
  const [mode, input, results] = args;
  if (args.length > (mode === '--report' ? 3 : 2)) fail('알 수 없는 실행 인자를 전달하지 마세요.');
  if (mode === '--resume' || mode === '--report') {
    const manifest = ownedManifest(path.resolve(input));
    if (mode === '--resume') return launch(manifest);
    const operator = operatorResults(results ? readJson(results) : {});
    const scan = await scanProductFiles(manifest.root, manifest.runId);
    const report = { version: 1, issue: 2724, runId: manifest.runId,
      sourceRevision: manifest.config.expectedRevision, userSidSha256: manifest.userSidSha256,
      fixtureIssue: manifest.config.sandboxIssueUrl, artifactSha256: manifest.artifacts.hashes,
      launches: manifest.launches, operator, scan,
      actualOsVerification: 'requires_operator_review',
      generatedAt: new Date().toISOString() };
    await writeJson(path.join(manifest.root, 'report.json'), report);
    console.log('report.json을 저장했습니다. operator_reported와 자동 파일 검사를 구분했습니다.');
    if (scan.result !== 'no_canary_observed' || manifest.launches.some((entry) => entry.captureTruncated)) process.exitCode = 2;
    return;
  }
  const config = validateConfig(readJson(path.resolve(input)));
  const artifacts = await preflight(config);
  if (mode === '--dry-run') {
    console.log(JSON.stringify({ result: 'ready_for_manual_launch', sourceRevision: config.expectedRevision,
      writes: 0, productProcessesStarted: 0, credentialCalls: 0, fixtureIssue: config.sandboxIssueUrl,
      stages: ['private helper copy', 'fresh AKRA_HOME/userData', 'visible product Electron',
        'manual native approval and OS lock', 'explicit resume', 'operator report and canary scan'] }, null, 2));
    return;
  }
  const runId = randomUUID().replaceAll('-', '');
  const prepared = JSON.parse(execFileSync(powershell(), ['-NoProfile', '-NonInteractive',
    '-File', STAGING, '-ExpectedSid', config.expectedUserSid, '-GatewayExecutable', config.gatewayExecutable,
    '-RunId', runId], { encoding: 'utf8', windowsHide: true }));
  const manifest = { version: 1, runId, root: prepared.root, config,
    userSidSha256: digest(config.expectedUserSid), artifacts, launches: [], createdAt: new Date().toISOString() };
  await writeJson(path.join(prepared.root, 'run.json'), manifest, 'wx');
  await writeJson(path.join(prepared.root, 'operator-results.template.json'),
    Object.fromEntries(STEPS.map((step) => [step, { outcome: 'not_run', reason: 'none' }])), 'wx');
  console.log('실행 기록: ' + path.join(prepared.root, 'run.json'));
  return launch(manifest);
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  cli(process.argv.slice(2)).catch((error) => {
    console.error('실행 보류: ' + error.message);
    process.exitCode = 1;
  });
}
