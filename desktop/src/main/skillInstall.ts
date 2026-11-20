// 설치된 스킬의 위치를 읽고 분류한다 (#2383). Akra 가 바꿀 수 있는 것은 아는 스킬 폴더 아래의 진짜 폴더뿐이다.
import { createHash } from "node:crypto";
import { lstatSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

import type { SkillIdentity, SkillRuntime } from "../shared/skills";

/** `<폴더>/<이 이름>/skills` 가 그 런타임의 스킬 폴더이다. 사용자 범위는 홈 폴더 아래, 프로젝트 범위는 작업 폴더 아래이다. */
const ROOT_PARENTS: Record<string, SkillRuntime> = { ".claude": "claude", ".codex": "codex", ".agents": "codex" };

export function skillKey(identity: SkillIdentity): string {
  return createHash("sha256").update([identity.runtime, identity.host, identity.scope, identity.installPath].join("\0")).digest("hex").slice(0, 32);
}

/** 경로의 모양만 보고 스킬 폴더(`…/skills`)를 찾는다. 파일시스템은 읽지 않는다. 아는 폴더 밖이면 null 이다. */
export function installRoot(runtime: SkillRuntime, installPath: string): string | null {
  if (!isAbsolute(installPath) || resolve(installPath) !== installPath) return null;
  for (let root = dirname(installPath); root !== dirname(root); root = dirname(root)) {
    if (basename(root) !== "skills" || ROOT_PARENTS[basename(dirname(root))] !== runtime) continue;
    // `.system` 같은 숨김 폴더 아래는 런타임이 관리한다.
    return relative(root, installPath).split(sep).some((segment) => segment.startsWith(".")) ? null : root;
  }
  return null;
}

export type InstallLocation = { management: "copy"; scope: "user" | "project" } | { management: "symlink" | "external" };

/**
 * 설치 폴더가 Akra 가 바꿀 수 있는 복사본인지 본다. 스킬 폴더에서 설치 폴더까지 링크가 하나라도 있으면 `symlink`
 * 이다: 링크를 고치거나 대체하지 않는다.
 */
export function classifyInstall(runtime: SkillRuntime, installPath: string, home: string): InstallLocation {
  const root = installRoot(runtime, installPath);
  if (!root) return { management: "external" };
  const steps = relative(root, installPath).split(sep);
  for (let index = 0; index <= steps.length; index += 1) {
    const stat = lstatSync(join(root, ...steps.slice(0, index)), { throwIfNoEntry: false });
    if (!stat) return { management: "external" };
    if (stat.isSymbolicLink()) return { management: "symlink" };
    if (!stat.isDirectory()) return { management: "external" };
  }
  const skillFile = lstatSync(join(installPath, "SKILL.md"), { throwIfNoEntry: false });
  if (!skillFile) return { management: "external" };
  if (!skillFile.isFile()) return { management: "symlink" };
  return { management: "copy", scope: dirname(dirname(root)) === resolve(home) ? "user" : "project" };
}

export interface FoundSkill {
  name: string;
  installPath: string;
  scope: "user" | "project";
}

/**
 * Claude Code 의 사용자·프로젝트 스킬 폴더를 디스크에서 읽는다. 설치 현황만 본다: 런타임에서 켜져 있는지는
 * 추정하지 않는다. 링크인 폴더도 목록에는 넣는다 (분류가 읽기 전용으로 만든다).
 */
export function scanClaudeSkills(home: string, cwd: string | null): FoundSkill[] {
  const bases: { base: string; scope: "user" | "project" }[] = [{ base: resolve(home), scope: "user" }];
  if (cwd && isAbsolute(cwd) && resolve(cwd) !== resolve(home)) bases.push({ base: resolve(cwd), scope: "project" });
  const found: FoundSkill[] = [];
  for (const { base, scope } of bases) {
    const root = join(base, ".claude", "skills");
    let names: string[];
    try {
      names = readdirSync(root).sort();
    } catch {
      continue;
    }
    for (const name of names) {
      if (name.startsWith(".")) continue;
      const installPath = join(root, name);
      try {
        if (statSync(join(installPath, "SKILL.md")).isFile()) found.push({ name, installPath, scope });
      } catch {
        // 스킬 폴더가 아니다 (파일이거나 SKILL.md 가 없다).
      }
    }
  }
  return found;
}
