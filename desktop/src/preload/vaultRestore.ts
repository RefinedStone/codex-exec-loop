// 복원 전용 window.akra 계약만 노출한다 (#2427). 일반 창의 Gateway·브라우저·파일 권한은 없다.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

import type { AkraVaultRestoreBridge, VaultRestoreBridge } from "../shared/bridge";
import { type VaultRestoreViewState, VAULT_RESTORE_CHANNELS } from "../shared/vaultRestore";

const bridge: VaultRestoreBridge = {
  ready: () => ipcRenderer.send(VAULT_RESTORE_CHANNELS.ready),
  onState(listener) {
    const handler = (_event: IpcRendererEvent, state: VaultRestoreViewState) => listener(state);
    ipcRenderer.on(VAULT_RESTORE_CHANNELS.state, handler);
    return () => ipcRenderer.off(VAULT_RESTORE_CHANNELS.state, handler);
  },
  unlock: () => ipcRenderer.send(VAULT_RESTORE_CHANNELS.unlock),
  lock: () => ipcRenderer.send(VAULT_RESTORE_CHANNELS.lock),
  review: () => ipcRenderer.send(VAULT_RESTORE_CHANNELS.review),
  reconnect: (mappings) => ipcRenderer.send(VAULT_RESTORE_CHANNELS.reconnect, mappings),
};

const surface: AkraVaultRestoreBridge = { vaultRestore: bridge };
contextBridge.exposeInMainWorld("akra", surface);
