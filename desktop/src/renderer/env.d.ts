import type { AkraBridge } from "../shared/bridge";

declare global {
  interface Window {
    /** preload 가 노출한 메시지 버스이다. Node API 는 쓸 수 없다. */
    akra: AkraBridge;
  }
}

export {};
