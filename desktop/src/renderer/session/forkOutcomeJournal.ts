// Each attempt owns a separate durable key. A later refusal or success must
// never erase an earlier uncertain attempt, even after renderer restart.
type ForkStorage = Pick<Storage, "length" | "key" | "getItem" | "setItem" | "removeItem">;
type AdmissionLock = <T>(name: string, enter: () => T) => Promise<T>;
const PREFIX = "akra.fork-pending.v2:";
const LEGACY_PREFIX = "akra.fork-pending.v1:";
export const EMPTY_FORK_SNAPSHOT = "[]";

function defaultStorage(): () => ForkStorage | null {
  // Node-only unit harnesses have no renderer. Their journal is per instance.
  const browser = (globalThis as { window?: { localStorage: ForkStorage } }).window;
  if (browser) return () => browser.localStorage;
  const data = new Map<string, string>();
  return () => ({
    get length() { return data.size; },
    key: index => [...data.keys()][index] ?? null,
    getItem: key => data.get(key) ?? null,
    setItem: (key, value) => { data.set(key, value); },
    removeItem: key => { data.delete(key); },
  });
}

function defaultAdmissionLock<T>(name: string, enter: () => T): Promise<T> {
  const browser = (globalThis as { window?: { navigator?: {
    locks?: { request<R>(name: string, callback: () => R): Promise<R> };
  } } }).window;
  // In supported renderers Web Locks serializes the read/check/write admission
  // among windows sharing the origin. Other profiles or external clients are
  // not covered. Without Web Locks the immediate recheck is best effort.
  const locks = browser?.navigator?.locks;
  return locks ? locks.request(name, enter) : Promise.resolve().then(enter);
}

export class ForkOutcomeJournal {
  constructor(
    private readonly storage: () => ForkStorage | null = defaultStorage(),
    private readonly admissionLock: AdmissionLock = defaultAdmissionLock,
  ) {}

  private requireStorage(): ForkStorage {
    const storage = this.storage();
    if (!storage) throw new Error("fork outcome storage unavailable");
    return storage;
  }

  private prefix(sourceId: string): string {
    return PREFIX + encodeURIComponent(sourceId) + ":";
  }

  snapshot(sourceId: string): string {
    const storage = this.requireStorage();
    const prefix = this.prefix(sourceId);
    const keys: string[] = [];
    for (let index = 0; index < storage.length; index++) {
      const key = storage.key(index);
      if (key?.startsWith(prefix)) keys.push(key);
    }
    // An older unresolved marker cannot be identified with a new attempt.
    // Preserve it as uncertainty rather than silently migrating it away.
    const legacy = storage.getItem(LEGACY_PREFIX + sourceId);
    if (legacy !== null) keys.push("legacy:" + legacy);
    return JSON.stringify([...new Set(keys)].sort());
  }

  has(sourceId: string): boolean {
    try { return this.snapshot(sourceId) !== EMPTY_FORK_SNAPSHOT; }
    catch { return true; }
  }

  async begin(sourceId: string, approvedSnapshot = EMPTY_FORK_SNAPSHOT): Promise<string | null> {
    const prefix = this.prefix(sourceId);
    return this.admissionLock(prefix, () => {
      // The approval refers to exactly the uncertainty the user saw before
      // asynchronous preflight. A new/changed marker requires a new decision.
      if (this.snapshot(sourceId) !== approvedSnapshot) return null;
      const attemptId = globalThis.crypto.randomUUID();
      const storage = this.requireStorage();
      const key = prefix + attemptId;
      storage.setItem(key, "1");
      if (storage.getItem(key) !== "1") throw new Error("fork outcome marker not retained");
      return attemptId;
    });
  }

  resolve(sourceId: string, attemptId: string): void {
    // Unique attempt keys avoid read/modify/write races when removing a result.
    try { this.requireStorage().removeItem(this.prefix(sourceId) + attemptId); }
    catch { /* Preserve uncertainty if removal fails. */ }
  }
}
