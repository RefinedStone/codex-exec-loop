// 문법 색을 넣는 언어의 목록이다 (P-12). 빌드가 `shiki/langs` 를 이 파일로 바꾼다 (`electron.vite.config.ts`).
// `shiki/langs` 는 모든 언어(약 12 MB)를 앱에 넣는다. 여기에 없는 언어의 파일은 색 없이 그린다.
type LanguageLoader = () => Promise<{ default: unknown }>;

export const bundledLanguages: Record<string, LanguageLoader> = {
  typescript: () => import("@shikijs/langs/typescript"),
  tsx: () => import("@shikijs/langs/tsx"),
  javascript: () => import("@shikijs/langs/javascript"),
  jsx: () => import("@shikijs/langs/jsx"),
  json: () => import("@shikijs/langs/json"),
  html: () => import("@shikijs/langs/html"),
  css: () => import("@shikijs/langs/css"),
  markdown: () => import("@shikijs/langs/markdown"),
  yaml: () => import("@shikijs/langs/yaml"),
  toml: () => import("@shikijs/langs/toml"),
  shellscript: () => import("@shikijs/langs/shellscript"),
  rust: () => import("@shikijs/langs/rust"),
  python: () => import("@shikijs/langs/python"),
  go: () => import("@shikijs/langs/go"),
  java: () => import("@shikijs/langs/java"),
  kotlin: () => import("@shikijs/langs/kotlin"),
  c: () => import("@shikijs/langs/c"),
  cpp: () => import("@shikijs/langs/cpp"),
  csharp: () => import("@shikijs/langs/csharp"),
  sql: () => import("@shikijs/langs/sql"),
};

/** 라이브러리가 파일 이름에서 고른 언어 이름 중, 위 목록의 다른 이름으로 받는 것이다. */
const ALIASES: Record<string, string> = { yml: "yaml", zsh: "shellscript", bash: "shellscript", sh: "shellscript", "objective-cpp": "c" };

/** 목록에 있는 언어이면 그 이름이고, 없으면 색 없는 글(`text`)이다. */
export function bundledLanguage(name: string): string {
  const language = ALIASES[name] ?? name;
  return Object.hasOwn(bundledLanguages, language) ? language : "text";
}
