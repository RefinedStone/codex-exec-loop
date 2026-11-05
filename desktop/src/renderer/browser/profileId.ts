// 브라우저 프로필 id 이다 (R3). main 의 세션 이름 규칙(소문자, 숫자, -, 32자)에 맞춘다.

/** 이름에서 프로필 id 를 만든다 (main 이 세션 이름으로 검증한다). 쓸 수 있는 글자가 없으면 "profile" 을 쓴다. */
export function profileIdFromName(name: string, existing: readonly string[]): string {
  const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 24) || "profile";
  let id = base;
  for (let suffix = 2; existing.includes(id); suffix += 1) id = `${base}-${suffix}`;
  return id;
}
