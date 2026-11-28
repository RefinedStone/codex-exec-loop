// 복원 전용 window.akra 계약만 노출한다 (#2427). 일반 창의 Gateway·브라우저·파일 권한은 없다.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

import type { AkraVaultRestoreBridge, VaultRestoreBridge } from "../shared/bridge";
import { type VaultRestoreViewState, VAULT_RESTORE_CHANNELS } from "../shared/vaultRestore";

const bridge: VaultRestoreBridge = {
  previewBackup: (generation, archive, recoveryKey) => ipcRenderer.invoke(VAULT_RESTORE_CHANNELS.backupPreview, generation, archive, recoveryKey),
  ready: () => ipcRenderer.send(VAULT_RESTORE_CHANNELS.ready),
  onState(listener) {
    const handler = (_event: IpcRendererEvent, state: VaultRestoreViewState, viewGeneration: number, backupAvailable?: boolean) => listener(state, viewGeneration, backupAvailable === true);
    ipcRenderer.on(VAULT_RESTORE_CHANNELS.state, handler);
    return () => ipcRenderer.off(VAULT_RESTORE_CHANNELS.state, handler);
  },
  unlock: (generation) => ipcRenderer.send(VAULT_RESTORE_CHANNELS.unlock, generation),
  lock: () => ipcRenderer.send(VAULT_RESTORE_CHANNELS.lock),
  review: (generation) => ipcRenderer.send(VAULT_RESTORE_CHANNELS.review, generation),
  reconnect: (generation, mappings) => ipcRenderer.send(VAULT_RESTORE_CHANNELS.reconnect, generation, mappings),
};

const surface: AkraVaultRestoreBridge = { vaultRestore: bridge };
contextBridge.exposeInMainWorld("akra", surface);
