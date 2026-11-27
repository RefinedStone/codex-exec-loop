// Persist only an opaque attempt ID per local source. Write before sending the
// non-idempotent RPC, so a renderer reload or crash cannot erase uncertainty.
type ForkStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
const PREFIX = "akra.fork-pending.v1:";

export class ForkOutcomeJournal {
  private readonly pending = new Map<string, string>();

  constructor(private readonly storage: () => ForkStorage | null = () =>
    (globalThis as { window?: { localStorage: ForkStorage } }).window?.localStorage ?? null) {}

  has(sourceId: string): boolean {
    if (this.pending.has(sourceId)) return true;
    try {
      return this.storage()?.getItem(PREFIX + sourceId) != null;
    } catch {
      // Inaccessible storage is not proof that there is no uncertain attempt.
      return true;
    }
  }

  begin(sourceId: string): string {
    const attemptId = globalThis.crypto.randomUUID();
    this.storage()?.setItem(PREFIX + sourceId, attemptId);
    this.pending.set(sourceId, attemptId);
    return attemptId;
  }

  resolve(sourceId: string, attemptId: string): void {
    // Compare the attempt before clearing it. This is not a cross-window lock;
    // same-renderer in-flight deduplication belongs to DesktopSession.
    try {
      const storage = this.storage();
      if (storage?.getItem(PREFIX + sourceId) === attemptId) storage.removeItem(PREFIX + sourceId);
      if (this.pending.get(sourceId) === attemptId) this.pending.delete(sourceId);
    } catch {
      // Keep the conservative warning if the known outcome cannot be recorded.
    }
  }
}
