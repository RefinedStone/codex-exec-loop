# Codex For Open Source Application

This document keeps the public application positioning for the Codex for Open Source program.
Update the usage metrics before submitting the form.

Official program pages:

- `https://openai.com/ko-KR/form/codex-for-oss/`
- `https://developers.openai.com/community/codex-for-oss`
- `https://developers.openai.com/codex/codex-for-oss-terms`

## Program Fit

The program looks for active open-source maintainers, repository usage, ecosystem importance, and
real maintainer workflows such as PR review, issue triage, release management, maintainer
automation, and security review.

Akra should be presented as a repository that directly implements those workflows on top of
`codex app-server`:

- native-first Rust TUI for long-lived Codex maintainer sessions
- accepted planning authority for queue and issue/task triage
- structured planning-tool automation for bounded task mutations
- parallel worktree pool for Codex-assisted PR delivery
- serialized GitHub PR, rebase integration, and cleanup path
- release packaging, npm publishing, checksum verification, and validation capture
- local-first safety controls: CSRF, Telegram allowlists, GitHub identity verification, explicit
  reset confirmation, isolated worktrees, and architecture-boundary tests

## Current Public Signals

As of 2026-06-04:

- Repository visibility: public
- GitHub stars: 0
- GitHub forks: 0
- Open GitHub issues: 0
- npm package: `@refinedstone/akra`
- npm downloads, last month: 523

Refresh before submission:

```bash
gh api repos/RefinedStone/codex-exec-loop \
  --jq '{stars: .stargazers_count, forks: .forks_count, open_issues: .open_issues_count, pushed_at: .pushed_at, visibility: .visibility}'

node -e 'fetch("https://api.npmjs.org/downloads/point/last-month/@refinedstone%2Fakra").then(r=>r.json()).then(j=>console.log(JSON.stringify(j, null, 2)))'
```

## Form Draft

### Name, Email, GitHub, Organization

- Last name: fill manually
- First name: fill manually
- Email: use the email on the ChatGPT account
- GitHub username: `RefinedStone` if that is the submitting maintainer account
- GitHub repository URL: `https://github.com/RefinedStone/codex-exec-loop`
- OpenAI organization ID: fill from the OpenAI Platform account

### Role

Recommended role option: `주 책임자`

Suggested text:

```text
주 책임자입니다. 저장소 구조, Rust 구현, release/npm/GitHub automation, planning/parallel-mode architecture, validation docs, PR integration flow를 직접 유지관리합니다.
```

### Why This Repository Fits

Use this for the 500-character field:

```text
Akra는 codex app-server 위에서 OSS 메인테이너의 반복 업무를 다루는 공개 Rust 프로젝트입니다. TUI, CLI, admin API, Telegram, planning-tool이 같은 서비스로 PR 리뷰 대기열, 이슈/작업 분류, release/validation, GitHub PR delivery를 운영합니다. GitHub 지표는 초기 단계지만 npm @refinedstone/akra는 최근 1개월 523 downloads이고, Codex를 이용한 maintainer workflow 자체를 재현·개선합니다.
```

### Interested Benefits

Recommended selections:

- `프로젝트에 사용할 API 크레딧`
- `Codex Security`, if the submitting maintainer has repository write/admin authority and wants
  security review support for this repository

### API Credit Plan

Use this for the 500-character field:

```text
API 크레딧은 Akra의 핵심 OSS 유지관리 루프에 사용하겠습니다. accepted planning에서 작업을 생성/검증하고, hidden planning worker로 queue-idle review를 수행하며, PR 리뷰 요약·이슈 분류·릴리스 체크리스트 작성·터미널 검증 로그 요약을 자동화합니다. 사용량은 repo-scoped SQLite authority와 validation 기록으로 추적하고, human-in-the-loop 승인과 GitHub identity guardrail을 유지하겠습니다.
```

### Additional Information

Use this for the optional 500-character field:

```text
Akra는 Codex를 대체하려는 도구가 아니라 Codex app-server를 native-first maintainer workflow에 연결하는 OSS 실험입니다. 로컬/loopback 우선, CSRF, Telegram allowlist, GitHub identity verification, worktree isolation, architecture-boundary tests를 갖추고 있어 Codex Security/API credit 지원 결과를 공개 구현과 문서로 환원하기 좋습니다.
```

## Submission Notes

- Do not claim broad adoption through GitHub stars; the current public star/fork signals are early.
- Lead with the program-relevant maintainer workflow implementation and npm usage signal.
- Keep all statements factual and verifiable from this repository.
- Do not submit secrets, private user data, or confidential maintainer details in the form.
