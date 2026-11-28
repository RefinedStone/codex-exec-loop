// 기본 E2E 목록은 발견된 파일에서 만들며 수동 시험만 명시적으로 제외한다.
export const MANUAL_E2E_SPECS = new Map([
  ["browserAutomation.mjs", "opens a visible Electron window (show: true)"],
  ["browserBridge.mjs", "needs AKRA_GATEWAY_BIN to point to a built native gateway"],
  ["storage-service-roundtrip.mjs", "needs AKRA_STORAGE_FIXTURE_BIN to point to a compiled Rust library test executable"],
  ["thread-fork-native.mjs", "needs AKRA_CODEX_ENTRY; uses two real app-servers and a disposable synthetic CODEX_HOME"],
]);

export function defaultE2eSpecs(fileNames) {
  return fileNames.filter((name) => name.endsWith(".mjs") && !MANUAL_E2E_SPECS.has(name)).sort();
}

export function parseE2eShard(value = "1/1") {
  const match = /^(\d+)\/(\d+)$/.exec(value);
  const index = Number(match?.[1]);
  const total = Number(match?.[2]);
  if (!Number.isSafeInteger(index) || !Number.isSafeInteger(total) || index < 1 || index > total) {
    throw new Error(`e2e: invalid shard "${value}"; expected 1-based index/total`);
  }
  return { index, total };
}

/** 모든 시험을 정확히 한 shard에 넣는다. 시간 표본이 없으면 같은 가중치로 분배한다. */
export function partitionE2eSpecs(specs, total, durationsMs = {}) {
  if (!Number.isSafeInteger(total) || total < 1 || total > specs.length) {
    throw new Error("e2e: shard count must be between 1 and the number of specs");
  }
  if (new Set(specs).size !== specs.length) throw new Error("e2e: duplicate specs");
  for (const [name, duration] of Object.entries(durationsMs)) {
    if (!Number.isFinite(duration) || duration <= 0) throw new Error(`e2e: invalid duration for ${name}`);
  }
  const samples = Object.values(durationsMs).sort((a, b) => a - b);
  const fallback = samples.length ? samples[Math.floor(samples.length / 2)] : 1;
  const weight = (name) => durationsMs[name] ?? fallback;
  const shards = Array.from({ length: total }, () => ({ specs: [], duration: 0 }));
  for (const spec of [...specs].sort((a, b) => weight(b) - weight(a) || (a < b ? -1 : a > b ? 1 : 0))) {
    const shard = shards.reduce((best, item) => item.duration < best.duration ? item : best);
    shard.specs.push(spec);
    shard.duration += weight(spec);
  }
  return shards.map(({ specs: assigned }) => assigned.sort());
}
