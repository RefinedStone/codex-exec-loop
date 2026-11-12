// 커널 패널의 연결이다 (plan 18 K3-6). 열린 스레드의 폴더가 속한 저장소의 커널에 Gateway 접속으로 붙고, 조회로
// 지금 상태를 읽은 뒤 이벤트로 갱신한다. 커널은 저장소당 하나이고, 붙어 있는 동안 그 저장소에서는 TUI 를 열 수
// 없다 (K3 §3.4). 그래서 패널이 열려 있을 때만 붙는다. 대화 기록은 여기서 다루지 않는다 (D-18).
import { JsonRpcError, type JsonRpcClient } from "../rpc/jsonRpcClient";
import {
  type AttachResponse,
  type AutoFollowSettings,
  KERNEL_METHOD,
  KERNEL_UNAVAILABLE,
  KERNEL_VIEW_QUERIES,
  type KernelEvent,
  type KernelViewAnswer,
  type ParallelState,
  type PlanningState,
  type ThreadActivity,
} from "./protocol";
import { sendKernelCommand } from "./sendKernelCommand";

export type KernelPhase = "idle" | "attaching" | "attached" | "failed";

export interface KernelView {
  phase: KernelPhase;
  /** 패널이 보여 줄 폴더이다. 없으면 열린 스레드가 없다. */
  workspace: string | null;
  /** 붙은 커널의 저장소 신원이다. */
  kernel: string | null;
  message: string | null;
  threads: ThreadActivity[];
  autoFollow: AutoFollowSettings | null;
  planning: PlanningState | null;
  parallel: ParallelState | null;
}

export const EMPTY_KERNEL_VIEW: KernelView = {
  phase: "idle",
  workspace: null,
  kernel: null,
  message: null,
  threads: [],
  autoFollow: null,
  planning: null,
  parallel: null,
};

export interface KernelStore {
  getKernel(): KernelView;
  setKernel(view: KernelView): void;
}

const CLIENT_NAME = "akra-desktop";

function failureMessage(error: unknown): string {
  if (error instanceof JsonRpcError && error.code === KERNEL_UNAVAILABLE) {
    return `커널에 붙을 수 없습니다. 같은 저장소에서 TUI 가 실행 중이면 TUI 를 닫으세요. (${error.message})`;
  }
  return error instanceof Error ? error.message : String(error);
}

export class KernelConnection {
  /** 패널이 원하는 폴더이다. 다시 연결되면 이 폴더로 다시 붙는다. */
  private wanted: string | null = null;
  private connected = false;
  /** 붙기마다 늘어난다. 늦게 끝난 이전 붙기와 조회의 결과는 반영하지 않는다. */
  private generation = 0;
  /** 종류별로 받은 이벤트 수이다. 조회 중에 같은 종류의 이벤트가 오면 더 새로운 이벤트를 남긴다. */
  private readonly eventCounts = new Map<string, number>();

  constructor(
    private readonly client: JsonRpcClient,
    private readonly store: KernelStore,
  ) {}

  /** 패널이 보여 줄 폴더를 정한다. 그 폴더의 저장소 커널에 붙고, null 이면 뗀다. */
  async show(workspace: string | null): Promise<void> {
    if (workspace === this.wanted) {
      return;
    }
    const previous = this.wanted;
    this.wanted = workspace;
    if (workspace === null) {
      this.generation += 1;
      this.store.setKernel(EMPTY_KERNEL_VIEW);
      if (previous !== null && this.connected) {
        await this.client.request(KERNEL_METHOD.detach, {}).catch(() => undefined);
      }
      return;
    }
    await this.attach();
  }

  /** Gateway 접속이 맺어지거나 끊겼다. 맺어지면 원하는 폴더로 다시 붙는다. 커널 붙음은 접속마다 따로이다. */
  connectionChanged(connected: boolean): void {
    this.connected = connected;
    this.generation += 1;
    void this.attach();
  }

  /** 지금 상태를 다시 읽고 planning 큐를 다시 읽게 한다. */
  async refresh(): Promise<void> {
    if (this.store.getKernel().phase !== "attached") {
      return;
    }
    const generation = this.generation;
    this.store.setKernel({ ...this.store.getKernel(), message: null });
    try {
      await this.readView(generation);
      await sendKernelCommand(this.client, { type: "refresh_planning" });
    } catch (error) {
      if (generation === this.generation) {
        this.showReadFailure(error);
      }
    }
  }

  /** `akra/kernel/event` 알림을 반영한다. */
  handleEvent(params: unknown): void {
    const event = (params as { event?: KernelEvent } | undefined)?.event;
    // 붙기 응답 전의 이벤트는 이전 커널의 것일 수 있다. 붙은 뒤에는 조회로 맞춘다.
    if (!event || this.store.getKernel().phase !== "attached") {
      return;
    }
    this.eventCounts.set(event.type, (this.eventCounts.get(event.type) ?? 0) + 1);
    if (event.type === "resync") {
      void this.readView(this.generation).catch((error: unknown) => this.showReadFailure(error));
      return;
    }
    this.apply(event);
  }

  private async attach(): Promise<void> {
    const workspace = this.wanted;
    if (workspace === null) {
      return;
    }
    if (!this.connected) {
      this.store.setKernel({ ...EMPTY_KERNEL_VIEW, workspace, message: "Gateway 연결을 기다립니다." });
      return;
    }
    const generation = ++this.generation;
    // 보던 상태는 붙는 동안 그대로 둔다. 같은 저장소의 다른 폴더로 옮기면 같은 커널에 다시 붙기 때문이다.
    const previous = this.store.getKernel();
    this.store.setKernel({ ...previous, phase: "attaching", workspace, message: null });
    let attached: AttachResponse;
    try {
      attached = await this.client.request<AttachResponse>(KERNEL_METHOD.attach, { workspace, client_name: CLIENT_NAME });
    } catch (error) {
      if (generation === this.generation) {
        this.store.setKernel({ ...EMPTY_KERNEL_VIEW, phase: "failed", workspace, message: failureMessage(error) });
      }
      return;
    }
    if (generation !== this.generation) {
      return;
    }
    const kept = previous.kernel === attached.kernel ? this.store.getKernel() : EMPTY_KERNEL_VIEW;
    this.store.setKernel({ ...kept, phase: "attached", workspace, kernel: attached.kernel, message: null });
    // 붙은 뒤의 실패는 붙음을 바꾸지 않는다. 커널에는 계속 붙어 있으므로 "새로 고침"으로 다시 읽을 수 있어야 한다.
    try {
      await this.readView(generation);
      await sendKernelCommand(this.client, { type: "refresh_planning" });
    } catch (error) {
      if (generation === this.generation) {
        this.showReadFailure(error);
      }
    }
  }

  private showReadFailure(error: unknown): void {
    const view = this.store.getKernel();
    if (view.phase === "attached") {
      this.store.setKernel({ ...view, message: `커널 상태를 읽지 못했습니다: ${failureMessage(error)}` });
    }
  }

  /** 조회 결과는 같은 이름의 이벤트와 모양이 같다. 조회하는 동안 같은 종류의 이벤트가 왔으면 조회 결과를 버린다. */
  private async readView(generation: number): Promise<void> {
    await Promise.all(
      KERNEL_VIEW_QUERIES.map(async (type) => {
        const seen = this.eventCounts.get(type) ?? 0;
        const event = await this.client.request<KernelViewAnswer>(KERNEL_METHOD.query, { query: { type } });
        if (generation === this.generation && (this.eventCounts.get(type) ?? 0) === seen) {
          this.apply(event);
        }
      }),
    );
  }

  private apply(event: KernelEvent): void {
    const view = this.store.getKernel();
    switch (event.type) {
      case "thread_activity":
        this.store.setKernel({ ...view, threads: event.threads });
        break;
      case "auto_follow":
        this.store.setKernel({ ...view, autoFollow: { max_turns: event.max_turns, paused: event.paused } });
        break;
      case "planning": {
        const { type: _type, ...planning } = event;
        this.store.setKernel({ ...view, planning });
        break;
      }
      case "parallel": {
        const { type: _type, ...parallel } = event;
        this.store.setKernel({ ...view, parallel });
        break;
      }
      default:
        // 세션 목록과 프롬프트 제출 진행은 이 패널이 보이지 않는다.
        break;
    }
  }
}
