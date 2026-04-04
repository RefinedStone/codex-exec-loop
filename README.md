# codex-exec-loop

`codex-exec-loop` 는 `codex exec` 와 `codex exec resume` 를 감싸서, 같은 Codex 세션을 자동으로 이어가는 실험용 CLI 입니다.

핵심 목표는 두 가지입니다.

- 새 세션을 열거나 기존 `session_id` 를 골라 같은 세션을 이어서 실행
- 각 턴이 끝나면 우리 쪽 follow-up 프롬프트를 같은 세션에 자동으로 다시 넣기

이 프로젝트는 PTY 주입이 아니라, Codex가 공식 제공하는 `exec` / `exec resume` 경로만 사용합니다.

## 상태

- 확인 일시: 2026-04-05
- 로컬 검증 대상 Codex CLI: `0.118.0`
- 완료 판정 기준: `codex exec --json` 의 `turn.completed`
- 성격: 공식 CLI 위에 얹는 비공식 오케스트레이터

## 저장소 구조

```text
.
├── README.md
├── examples/
│   ├── followup_prompt.txt
│   └── initial_prompt.txt
├── pyproject.toml
└── src/
    └── codex_exec_loop/
        ├── __init__.py
        ├── __main__.py
        ├── cli.py
        ├── runner.py
        └── sessions.py
```

## 요구사항

- Python 3.10+
- Codex CLI 설치
- Codex 로그인 완료
- `~/.codex/history.jsonl` 와 `~/.codex/sessions/` 에 접근 가능한 로컬 환경

## 설치

```bash
cd /home/akra/codex-exec-loop
python3 -m venv .venv
. .venv/bin/activate
pip install -U pip
pip install -e .
```

## 빠른 시작

새 세션 시작 후 1번 자동 follow-up:

```bash
cd /home/akra/codex-exec-loop
. .venv/bin/activate

codex-exec-loop \
  --cwd /home/akra/codex-exec-loop \
  --prompt-file examples/initial_prompt.txt \
  --followup-file examples/followup_prompt.txt \
  --max-auto-turns 1 \
  --full-auto \
  --transcript logs/demo.log
```

기존 세션 이어서 시작:

```bash
codex-exec-loop \
  --mode existing \
  --session-id 019d5a13-afb3-7850-a416-523662e99b3a \
  "방금 답변 기준으로 다음 작업 하나만 이어서 진행해 주세요."
```

최근 세션 목록 확인:

```bash
codex-exec-loop sessions --limit 10
```

## 실제 작업물 테스트

가장 눈에 잘 보이는 테스트는 Codex가 실제 파일을 만들고, 자동 follow-up 턴에서 그 파일에 내용을 이어붙이게 하는 방식입니다.

아래 스크립트는 실행할 때마다 새 Markdown 파일을 `artifacts/` 아래에 생성합니다.

```bash
cd /home/akra/codex-exec-loop
. .venv/bin/activate

./scripts/run_artifact_smoke_test.sh
```

정상이라면:

- `artifacts/SMOKE_WORK_PRODUCT_<timestamp>.md` 파일이 생김
- 파일 안에 `turn-1` 과 `followup-1` 이 둘 다 들어감
- `logs/artifact-smoke-<timestamp>.log` 에 두 번의 `turn.completed` 가 남음

스크립트 끝에는 생성된 파일 경로, 파일 내용, 로그 경로를 같이 출력합니다.

## 동작 방식

1. 로컬 `~/.codex/history.jsonl` 에서 최근 세션 목록을 읽습니다.
2. 필요하면 `~/.codex/sessions/**/rollout-*.jsonl` 의 `session_meta` 로 세션 존재를 다시 검증합니다.
3. 첫 턴은 `codex exec --json` 또는 `codex exec resume --json` 으로 실행합니다.
4. JSON 이벤트를 읽다가 `turn.completed` 가 오면 턴 종료로 판정합니다.
5. follow-up 템플릿을 렌더링합니다.
6. 같은 `session_id` 로 `codex exec resume --json SESSION_ID FOLLOWUP` 을 다시 실행합니다.

## CLI 사용법

기본 명령은 `run` 이고, 생략할 수 있습니다.

```bash
codex-exec-loop run [PROMPT]
codex-exec-loop [PROMPT]
```

주요 옵션:

- `--mode {new,existing}`
  - 생략하면 TTY 에서 `new` / `existing` 를 묻습니다.
- `--session-id`
  - 기존 세션 모드에서 사용할 세션 ID
- `--prompt-file`
  - 첫 프롬프트를 파일에서 읽습니다.
- `--followup`
  - follow-up 텍스트 직접 지정
- `--followup-file`
  - follow-up 템플릿 파일
- `--max-auto-turns`
  - 첫 턴 이후 자동 후속 턴 수
- `--cwd`
  - 새 세션 시작 시 `codex exec -C ...` 에 전달할 디렉터리
- `--full-auto`
  - Codex 쪽 실행을 `--full-auto` 로 돌립니다.
- `--transcript`
  - raw JSONL 과 wrapper 로그를 같이 남깁니다.
- `--skip-git-repo-check` / `--no-skip-git-repo-check`
  - 기본값은 `--skip-git-repo-check` 입니다.

`sessions` 서브커맨드는 최근 세션과 마지막 입력 미리보기를 보여줍니다.

```bash
codex-exec-loop sessions --limit 20
```

## Follow-up 템플릿 변수

follow-up 템플릿에는 아래 변수를 쓸 수 있습니다.

- `{auto_turn}`: 현재 자동 후속 턴 번호. `1` 부터 시작
- `{max_auto_turns}`: 전체 자동 후속 턴 수
- `{session_id}`: 현재 세션 ID
- `{last_message}`: 직전 Codex 마지막 답변

예시:

```text
대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 답변:
{last_message}

이 내용을 바탕으로 다음 작업 1개만 이어서 진행하세요.
```

## 인터랙티브 모드

아무 옵션 없이 실행하면 간단한 선택형 프롬프트가 뜹니다.

```bash
codex-exec-loop
```

여기서:

- `new` 또는 `existing` 을 고르고
- `existing` 이면 최근 세션 목록을 보고 번호 또는 `session_id` 를 입력하고
- 그 세션 ID 가 실제로 존재하는지 검증한 뒤
- 보낼 프롬프트를 입력하게 됩니다

긴 프롬프트는 `--prompt-file` 사용을 권장합니다.

## 한계

- `codex exec --json` 이벤트 형식이 바뀌면 파서가 수정되어야 합니다.
- 현재 완료 기준은 `turn.completed` 하나입니다.
- 출력은 스트리밍 토큰 단위가 아니라 턴 단위 메시지 중심입니다.
- `resume` 은 기존 세션의 컨텍스트를 이어가지만, 현재 래퍼는 Codex 내부 계획 상태를 별도로 해석하지 않습니다.
- 세션 목록은 로컬 히스토리 파일 기준입니다.
  - 다른 머신이나 다른 `CODEX_HOME` 에 있는 세션은 바로 보이지 않습니다.
- `--full-auto` 를 쓰지 않으면 Codex 쪽 승인 흐름에서 자동 루프가 멈출 수 있습니다.
- 새 세션의 trust / repo 정책 자체를 우회하지는 않습니다.
  - 필요하면 `--skip-git-repo-check` 를 유지하고, Codex 쪽 trust 설정은 사용 환경에서 따로 맞춰야 합니다.

## 권장 사용법

- 짧은 실험은 `--max-auto-turns 1` 로 시작하세요.
- 긴 follow-up 보다는 결정적인 후속 지시 2~5줄이 안정적입니다.
- 로그가 필요하면 항상 `--transcript` 를 켜는 편이 좋습니다.
- 기존 세션을 이어갈 때는 먼저 `codex-exec-loop sessions` 로 최근 세션을 확인하세요.
- 실제 작업물 검증은 `./scripts/run_artifact_smoke_test.sh` 로 먼저 확인하세요.

## 검증

로컬에서 아래까지 확인했습니다.

- `python -m py_compile src/codex_exec_loop/*.py`
- `codex-exec-loop --help`
- `codex-exec-loop sessions --limit 3`
- `codex exec --json` 에서 `turn.completed` 감지
- 새 `thread_id` 를 `codex exec resume --json SESSION_ID ...` 에 바로 재사용 가능함 확인

실제 follow-up 자동 루프는 Codex 로그인 상태와 실행 권한에 따라 달라지므로, 사용 환경에서 한 번 더 현장 검증하는 것을 권장합니다.
