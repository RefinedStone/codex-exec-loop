// 생성 타입의 codex 버전과 Gateway 가 실행한 codex 버전을 비교한다 (04 §9). 다르면 경고만 한다.

const VERSION_PATTERN = /\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?/;

export function extractCodexVersion(text: string | null | undefined): string | null {
  return text ? (VERSION_PATTERN.exec(text)?.[0] ?? null) : null;
}

export interface CodexVersionCheck {
  generated: string | null;
  gateway: string | null;
  matches: boolean;
}

export function checkCodexVersion(generatedText: string, gatewayText: string | null): CodexVersionCheck {
  const generated = extractCodexVersion(generatedText);
  const gateway = extractCodexVersion(gatewayText);
  return { generated, gateway, matches: generated !== null && generated === gateway };
}
