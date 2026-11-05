// 알림을 바로 그리지 않고 모았다가 화면 프레임마다 한 번 반영한다 (P3 §7). 순서는 그대로 지킨다.

export type FrameScheduler = (callback: () => void) => void;

const defaultScheduler: FrameScheduler = (callback) => {
  // 시험(Node)에는 requestAnimationFrame 이 없으므로 있을 때만 쓴다.
  const requestFrame = (globalThis as { requestAnimationFrame?: (callback: () => void) => number })
    .requestAnimationFrame;
  if (requestFrame) {
    requestFrame(() => callback());
  } else {
    setTimeout(callback, 16);
  }
};

export class FrameBatcher<T> {
  private queue: T[] = [];
  private scheduled = false;

  constructor(
    private readonly flush: (batch: T[]) => void,
    private readonly schedule: FrameScheduler = defaultScheduler,
  ) {}

  push(value: T): void {
    this.queue.push(value);
    if (!this.scheduled) {
      this.scheduled = true;
      this.schedule(() => this.drain());
    }
  }

  /** 모인 값을 지금 반영한다. 스레드를 바꾸기 전에 남은 알림을 버리지 않게 쓴다. */
  drain(): void {
    this.scheduled = false;
    if (this.queue.length === 0) {
      return;
    }
    const batch = this.queue;
    this.queue = [];
    this.flush(batch);
  }

  clear(): void {
    this.queue = [];
  }
}
