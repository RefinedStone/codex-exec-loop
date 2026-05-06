# Parallel Mode MUD Timeline UI Pack Research

## Summary

Parallel Mode의 다음 UI Pack은 "운영형 MUD timeline"으로 잡는 것이 적합하다. Slot은 room/lane, agent는 actor, distributor는 exit corridor, session history는 event timeline으로 표시한다. 목표는 화면을 화려하게 만드는 것이 아니라, 운영자가 병렬 작업의 현재 병목을 3초 안에 찾는 것이다.

## Evidence

- GitHub Actions는 workflow run마다 progress graph를 만들고, job status icon과 dependency line을 보여 준 뒤 job log는 선택한 job에서 연다.
- CircleCI는 pipeline, workflow, job 상태를 계층화하고 rerun, cancel, approve, job step output/log 접근을 같은 운영 맥락에 둔다.
- Langfuse는 agent graph를 multi-step reasoning과 agent interaction 디버깅용으로 제공하고, trace timeline을 latency bottleneck, parallelism, nested reasoning 확인용으로 설명한다.
- AgentOps는 session waterfall에서 좌측 timeline과 우측 selected event detail을 결합한다.
- Logfire는 AI workload에서 conversation panel, token/cost tracking, tool call argument/response/latency, streaming chunk, multi-turn flow를 관측 대상으로 둔다.

## Recommended UI Shape

```text
+------------------------------------------------------+
| parallel ON | readiness OK | pool 1/3 running | q:2  |
+---------------+----------------------+---------------+
| slot-1 RUN    | slot-2 IDLE          | slot-3 BLOCK  |
| task: UI pack | branch: slot-2       | stale lease   |
| age: 04m12s   | ready                | fix cleanup   |
+---------------+----------------------+---------------+
| timeline: slot-1 / session akra-...                  |
| 10:04 assigned -> 10:05 thread -> 10:06 running      |
| 10:09 reported_complete -> ledger_refreshing         |
| detail: validation summary / authority refresh / PR  |
+------------------------------------------------------+
```

## Implementation Notes

- Start in projection/copy before rendering. The current owner file is `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/supersession.rs`.
- Reuse session lifecycle terms already persisted by `src/application/service/parallel_mode/session_detail.rs`: `assigned`, `starting`, `running`, `reported_complete`, `ledger_refreshing`, `commit_ready`, `merge_queued`.
- Keep topology and chronology separate: slot/lane board for current parallelism, selected timeline for transition history.
- Keep key footer conservative. `docs/design/07-tui-layered-architecture-and-aesthetic-contract.md` requires displayed shortcuts to match implemented input paths.
- Use fixed-width row fields and truncate branch/task/path labels to protect narrow TUI snapshots.

## Delivered First Slice

The first slice is shipped as a read-only Supersession timeline projection:

- Timeline lines are added to the existing supervisor detail view model in
  `src/adapter/inbound/tui/app/shell_presentation/overlays/popup/supersession.rs`.
- Selected session lifecycle renders as compact event arrows under the current pool/roster sections.
- No new commands or mutable controls were added.
- Coverage is pinned by supersession popup copy tests and a narrow inline shell rendering contract
  test that keeps the selected timeline visible.

## References

- Lazyweb report: `.lazyweb/design-research/parallel-mud-timeline-ui-pack-2026-05-06/report.md`
- GitHub Docs: https://docs.github.com/en/actions/how-tos/monitor-workflows/use-the-visualization-graph
- CircleCI Docs: https://circleci.com/docs/guides/about-circleci/introduction-to-the-circleci-web-app/
- Langfuse Agent Graphs: https://langfuse.com/docs/observability/features/agent-graphs
- Langfuse Trace Timeline View: https://langfuse.com/changelog/2024-06-12-timeline-view
- AgentOps Dashboard: https://docs.agentops.ai/v2/usage/dashboard-info
- Pydantic Logfire AI Observability: https://pydantic.dev/docs/logfire/get-started/ai-observability/
