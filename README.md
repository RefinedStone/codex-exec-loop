# codex-exec-loop

`codex-exec-loop` 는 `codex exec` 와 `codex exec resume` 를 감싸서, 같은 Codex 세션을 자동으로 이어가는 CLI 입니다.

핵심은 세 가지입니다.

- 새 세션 또는 기존 `session_id` 를 선택해서 같은 세션 흐름 유지
- 각 턴 종료를 `codex exec --json` 의 `turn.completed` 로 감지
- 후속 프롬프트를 같은 세션에 `codex exec resume` 으로 다시 넣기

이 프로젝트는 PTY 주입이 아니라, Codex CLI의 공식 non-interactive 경로 위에서 동작합니다.

## 상태

- 확인 일시: 2026-04-05
- 로컬 검증 대상 Codex CLI: `0.118.0`
- 공식 문서 기준으로 반영한 포인트:
  - `codex exec --json`
  - `codex exec resume`
  - `-o, --output-last-message`
  - `--output-schema`

## 저장소 구조

```text
.
├── README.md
├── examples/
│   ├── followup_prompt.txt
│   ├── followups/
│   │   ├── bugfix.txt
│   │   ├── docs.txt
│   │   ├── next_task.txt
│   │   └── plan_queue.txt
│   └── initial_prompt.txt
├── pyproject.toml
├── scripts/
│   └── run_artifact_smoke_test.sh
└── src/
    └── codex_exec_loop/
        ├── __init__.py
        ├── __main__.py
        ├── cli.py
        ├── runner.py
        ├── runs.py
        ├── sessions.py
        └── verifier.py
```

## 요구사항

- Python 3.10+
- Codex CLI 설치
- Codex 로그인 완료
- 로컬 `~/.codex/history.jsonl` / `~/.codex/sessions/` 접근 가능

## 설치

```bash
cd /home/akra/codex-exec-loop
python3 -m venv .venv
. .venv/bin/activate
PYTHONPATH=/usr/lib/python3/dist-packages python -m pip install --no-build-isolation -e .
```

현재 WSL/오프라인 환경에서는 위 방식이 가장 안전합니다.

## 가장 간단한 실행

대화형 입력 없이 새 세션으로 1회 자동 follow-up:

```bash
cd /home/akra/codex-exec-loop
. .venv/bin/activate

codex-exec-loop \
  --yes \
  --max-auto-turns 1 \
  --followup 'Reply with the single word AGAIN.' \
  --output-dir logs/demo-run \
  'Reply with the single word OK.'
```

정상이면:

- 첫 턴에서 `OK`
- 같은 `session_id` 로 자동 resume
- 두 번째 턴에서 `AGAIN`
- `logs/demo-run/summary.json`
- `logs/demo-run/transcript.log`
- `logs/demo-run/turns/turn-01-last-message.txt`
- `logs/demo-run/turns/turn-02-last-message.txt`

## 실제 작업물 테스트

실제 파일이 생성되고, 자동 후속 턴에서 같은 파일에 내용이 추가되는 스모크 테스트입니다.

```bash
cd /home/akra/codex-exec-loop
. .venv/bin/activate

./scripts/run_artifact_smoke_test.sh
```

정상이면:

- `artifacts/SMOKE_WORK_PRODUCT_<timestamp>.md` 생성
- 파일 안에 `- turn-1` 과 `- followup-1` 둘 다 존재
- `logs/artifact-smoke-<timestamp>/summary.json` 생성
- `codex-exec-loop verify` 가 성공

## 사용법

기본 명령은 `run` 이며 생략할 수 있습니다.

```bash
codex-exec-loop run [PROMPT]
codex-exec-loop [PROMPT]
```

### 새 세션 시작

```bash
codex-exec-loop \
  --yes \
  --mode new \
  --cwd /path/to/project \
  --prompt-file examples/initial_prompt.txt \
  --followup-file examples/followups/next_task.txt \
  --max-auto-turns 1 \
  --full-auto \
  --output-dir logs/run-001
```

### 기존 세션 이어서 시작

```bash
codex-exec-loop sessions --limit 10

codex-exec-loop \
  --yes \
  --mode existing \
  --session-id 019d5a7a-cee0-7e33-8fed-41819faa07f4 \
  --followup-strategy plan-queue \
  --max-auto-turns 1 \
  "방금 결과 기준으로 다음 작업 하나만 이어서 진행하세요."
```

### 최근 세션 보기

```bash
codex-exec-loop sessions --limit 20
codex-exec-loop sessions --limit 20 --query plan
codex-exec-loop sessions --limit 5 --json
```

### 작업 결과 검증

```bash
codex-exec-loop verify \
  --summary logs/demo-run/summary.json \
  --must-exist artifacts/example.md \
  --must-contain 'artifacts/example.md::- turn-1' \
  --must-contain 'artifacts/example.md::- followup-1' \
  --expect-changed artifacts/example.md \
  --show-file artifacts/example.md
```

## 주요 옵션

- `--yes`, `--non-interactive`
  - 모드 선택과 프롬프트 입력을 묻지 않습니다.
- `--mode {new,existing}`
  - 세션 시작 방식 지정
- `--session-id`
  - 기존 세션 모드에서 사용할 세션
- `--prompt-file`
  - 첫 프롬프트 파일
- `--followup`
  - 후속 프롬프트 텍스트 직접 지정
- `--followup-file`
  - 후속 프롬프트 템플릿 파일
- `--followup-strategy`
  - 내장 후속 전략 선택
  - `last-message`, `plan-queue`, `bugfix`, `docs`, `next-task`
- `--max-auto-turns`
  - 첫 턴 뒤에 몇 번 자동 resume 할지
  - `inf`, `infinite`, `unlimited`, `-1` 도 지원
- `--stop-on-keyword`
  - 마지막 답변에 특정 키워드가 나오면 다음 follow-up 중단
- `--stop-when-no-files-changed`
  - 해당 턴의 `file_change` 이벤트가 없으면 중단
- `--fallback-new-on-missing-session`
  - 기존 세션 resume 실패 시 새 세션으로 fallback
- `--output-dir`
  - `summary.json`, `transcript.log`, `turn-XX-last-message.txt` 저장
- `--output-schema`
  - Codex CLI의 `--output-schema` 를 그대로 전달
- `--transcript`
  - 별도 transcript 파일 경로

## Follow-up 템플릿 변수

후속 프롬프트 템플릿에는 아래 변수를 쓸 수 있습니다.

- `{auto_turn}`
- `{max_auto_turns}`
- `{session_id}`
- `{last_message}`

## 내장 follow-up 전략

### `last-message`

직전 답변을 인용하고 다음 작업 1개를 이어서 하게 합니다.

### `plan-queue`

`plan_priority_queue.md` 에 후보를 적고, 가장 우선순위 높은 1개를 바로 진행하게 합니다.

### `bugfix`

직전 변경분 기준으로 남은 버그나 리스크 1개만 고치게 합니다.

### `docs`

직전 작업을 바탕으로 README 또는 사용자 문서 보강을 지시합니다.

### `next-task`

직전 결과 기준으로 다음 작업 1개만 이어서 하게 하는 최소 템플릿입니다.

## Structured Run Output

`--output-dir logs/run-001` 를 주면 아래 파일이 생성됩니다.

- `summary.json`
  - 세션 ID
  - stop reason
  - 각 턴의 prompt / usage / file_changes
- `transcript.log`
  - raw JSONL 이벤트와 wrapper 로그
- `last-session-id.txt`
  - 마지막 세션 ID
- `turns/turn-01-last-message.txt`
  - Codex CLI `-o` 로 저장한 마지막 메시지

즉, 공식 문서에 있는 `-o, --output-last-message` 를 wrapper 내부에서 turn별로 활용합니다.

## 무한 자동 턴

무한 반복도 지원합니다.

```bash
codex-exec-loop \
  --yes \
  --max-auto-turns infinite \
  --stop-when-no-files-changed \
  --followup-strategy next-task \
  --output-dir logs/infinite-run \
  "현재 작업 기준으로 다음 작업을 계속 이어서 진행하세요."
```

또는 아래 별칭도 같습니다.

```bash
--max-auto-turns inf
--max-auto-turns unlimited
--max-auto-turns -1
```

권장사항:

- 무한 모드에서는 `--stop-when-no-files-changed` 또는 `--stop-on-keyword` 를 같이 쓰세요.
- stop rule 없이 무한 모드를 쓰면 사용자가 직접 중단하기 전까지 계속 돌 수 있습니다.

## 세션 선택 UX

`--mode existing` 에서 interactive 모드로 실행하면:

- 최근 세션 목록 출력
- 번호 입력 가능
- 정확한 `session_id` 입력 가능
- `/검색어` 로 미리보기 / cwd / session_id 필터 가능
- 로컬 기록에서 실제 존재 여부 검증

## 한계

- `codex exec --json` 이벤트 형식이 바뀌면 파서 수정이 필요합니다.
- `stop-when-no-files-changed` 는 `file_change` 이벤트 기준입니다.
  - 파일을 바꿨는데 Codex가 해당 이벤트를 내리지 않는 특수 경우는 놓칠 수 있습니다.
- 세션 존재 검증은 로컬 history / rollout 기준이며, 원격 유효성 검사 API는 사용하지 않습니다.
- `fallback-new-on-missing-session` 은 resume 실패 시 새 세션으로 넘어가는 보완책일 뿐, 기존 컨텍스트를 복구하지는 않습니다.
- MCP 기반 OpenAI docs 조회는 현재 세션 재시작 전까지 활성화되지 않을 수 있습니다.
  - 이 경우 공식 OpenAI 웹 문서를 fallback 으로 참조했습니다.

## OpenAI 문서 기준 메모

OpenAI 공식 Codex 문서와 Prompting Guide 기준으로 현재 구조에서 특히 의미 있는 포인트는 아래 두 가지였습니다.

- non-interactive 자동화는 `codex exec` / `codex exec resume` / `--json` 조합이 기본 경로
- 마지막 응답 파일은 `-o, --output-last-message` 로 따로 저장하는 편이 더 안정적

또한 Stop hook continuation 도 공식 경로이지만, 이 프로젝트는 사용자가 원한 `session_id 기반 exec/resume 루프` 를 중심으로 유지했습니다.

## 검증

로컬에서 아래까지 확인했습니다.

- `python -m py_compile src/codex_exec_loop/*.py`
- `codex-exec-loop --help`
- `codex-exec-loop sessions --limit 3`
- `codex-exec-loop verify --help`
- `OK -> AGAIN` 자동 resume 스모크 테스트
- 실제 파일 생성 + 후속 파일 수정 스모크 테스트
