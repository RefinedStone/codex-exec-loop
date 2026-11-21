/** 취소된 요청이 다음 창의 사용자 확인으로 넘어가지 않게 순서를 유지한다. */
export class SerialQueue {
  private tail: Promise<unknown> = Promise.resolve();

  run<T>(work: () => Promise<T>): Promise<T> {
    const result = this.tail.then(work, work);
    this.tail = result.catch(() => undefined);
    return result;
  }
}
