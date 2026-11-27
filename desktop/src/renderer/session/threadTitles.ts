// #2913: display-only fallback. Explicit app-server names are authoritative.
// No model call, name write, or durable prompt copy is made by this module.
export const UNTITLED_THREAD = "(제목 없음)";
const MAX_GRAPHEMES = 36;
const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: "grapheme" });
const GREETING = /^(안녕(?:하세요|하십니까)?|하이|반가워(?:요)?|hello|hi|hey|thanks|thank you|고마워(?:요)?|감사합니다)[\s.!?~]*$/iu;
// This conservative display guard is not a substitute for the shared disclosure policy.
const SENSITIVE = /(?:bearer\s+\S+|-----BEGIN [A-Z ]+PRIVATE KEY-----|(?:api[_ -]?key|token|password|secret|비밀번호|인증키)\s*[:=]|https?:\/\/[^\s/@]+:[^\s/@]+@|\bsk-[A-Za-z0-9_-]{12,})/iu;
const CONTROLS = /[\p{Cc}\p{Cf}]/gu;

export function provisionalThreadTitle(text: string): string | null {
  if (SENSITIVE.test(text)) return null;
  const line = text.split(/\r?\n/u).map((value) => value.trim()).find(Boolean);
  if (!line || GREETING.test(line) || /^\s*(?:```|~~~)/u.test(line)) return null;
  const clean = line.replace(CONTROLS, (value) => value === "\u200d" ? value : " ").replace(/\s+/gu, " ").trim();
  if (!clean) return null;
  const parts = [];
  for (const { segment } of GRAPHEMES.segment(clean)) {
    if (parts.length === MAX_GRAPHEMES) return parts.slice(0, MAX_GRAPHEMES - 1).join("") + "…";
    parts.push(segment);
  }
  return parts.join("");
}

export function threadDisplayTitle(name: string | null | undefined, preview: string): string {
  return name?.trim() ? name : provisionalThreadTitle(preview) ?? UNTITLED_THREAD;
}

/** A single renderer's non-authoritative fallback for threads it just created. */
export class ProvisionalThreadTitles {
  private readonly eligible = new Set<string>();
  private readonly displayed = new Map<string, string>();

  register(threadId: string, name: string | null | undefined, preview: string): void {
    if (!name?.trim() && !preview.trim() && !this.displayed.has(threadId)) this.eligible.add(threadId);
  }

  accepted(threadId: string, text: string): string | null {
    if (!this.eligible.has(threadId)) return null;
    const title = provisionalThreadTitle(text);
    if (!title) return null;
    this.eligible.delete(threadId);
    this.displayed.set(threadId, title);
    return title;
  }

  stopGenerating(threadId: string): void { this.eligible.delete(threadId); }

  forget(threadId: string): void {
    this.eligible.delete(threadId);
    this.displayed.delete(threadId);
  }

  preview(threadId: string, name: string | null | undefined, providerPreview: string): string {
    if (name?.trim()) {
      this.forget(threadId);
      return providerPreview;
    }
    return this.displayed.get(threadId) ?? providerPreview;
  }

  clear(): void { this.eligible.clear(); this.displayed.clear(); }
}
