// Electron lock/unlock events are documented on Windows and macOS only.
// A positive idle-state observation does not establish the missing event
// boundary on another OS. Keep Desktop plaintext viewing unavailable there.
import type { VaultRestoreHold } from "../shared/vaultRestore";

/** Main-owned power admission check; failures are never treated as unlocked. */
export function vaultRestorePowerHold(
  platform: NodeJS.Platform,
  readIdleState: () => unknown,
): VaultRestoreHold | null {
  if (platform !== "win32" && platform !== "darwin") return "os_lock_unsupported";
  try {
    switch (readIdleState()) {
      case "locked": return "os_locked";
      case "active":
      case "idle": return null;
      default: return "os_lock_unavailable";
    }
  } catch {
    return "os_lock_unavailable";
  }
}
