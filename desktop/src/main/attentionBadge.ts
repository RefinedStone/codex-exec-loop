import { app, nativeImage, type BrowserWindow } from "electron";

type Demand = {readonly kernel: string; readonly count: number};
const reports = new Map<BrowserWindow, Demand>();
const digits = ["111101101101111", "010110010010111", "111001111100111", "111001111001111", "101101111001001",
  "111100111001111", "111100111101111", "111001001001001", "111101111101111", "111101111001111"];

// Windows requires a bitmap for the taskbar overlay, not a Dock badge string.
function overlay(count: number) {
  const chars = String(count);
  const size = Math.max(20, chars.length * 4 + 2);
  const bitmap = Buffer.alloc(size * size * 4);
  const scale = Math.max(1, Math.floor((size - 2) / Math.max(5, chars.length * 4 - 1)));
  const left = Math.floor((size - (chars.length * 4 - 1) * scale) / 2);
  const top = Math.floor((size - 5 * scale) / 2);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const col = Math.floor((x - left) / scale), row = Math.floor((y - top) / scale);
    const glyph = digits[Number(chars[Math.floor(col / 4)])];
    const ink = col >= 0 && col % 4 < 3 && row >= 0 && row < 5 && glyph?.[row * 3 + col % 4] === "1";
    bitmap.writeUInt32LE(ink ? 0xffffffff : 0xffa32f36, (y * size + x) * 4);
  }
  return nativeImage.createFromBitmap(bitmap, {width:size, height:size});
}

export function reportAttention(owner: BrowserWindow, windows: ReadonlySet<BrowserWindow>, kernel: unknown, count: unknown): void {
  // Reinsert valid reports at the end: Map.set on an existing key keeps its old position.
  reports.delete(owner);
  if (typeof kernel === "string" && kernel.length > 0 && typeof count === "number" && Number.isSafeInteger(count) && count >= 0) {
    reports.set(owner, {kernel, count});
  }
  // Multiple windows may be attached to one kernel. Its latest current report counts once.
  const counts = new Map<string, number>();
  for (const report of reports.values()) counts.set(report.kernel, report.count);
  const total = [...counts.values()].reduce((sum, n) => sum + n, 0);
  if (process.platform === "win32") {
    const icon = total ? overlay(total) : null;
    for (const window of windows) if (!window.isDestroyed()) window.setOverlayIcon(icon, total ? `${total} responses required` : "");
  } else {
    app.setBadgeCount(total);
  }
}
