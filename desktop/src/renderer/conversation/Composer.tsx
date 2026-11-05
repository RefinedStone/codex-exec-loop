// 입력창 (P3 §6.2). Enter 는 보내기, Shift+Enter 는 줄바꿈이다. 턴이 진행 중이면 보내기는 steer 가 되고
// 중단 버튼이 보인다. 연결이 끊기면 입력을 막는다 (§6.5).
import { type KeyboardEvent, useEffect, useState } from "react";

import { useDesktopStore } from "../session/desktopStore";
import { isTurnInProgress } from "./transcript";

export interface ComposerActions {
  sendMessage(text: string): Promise<void>;
  interrupt(): Promise<void>;
}

export function Composer({ actions }: { actions: ComposerActions }) {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const connected = useDesktopStore((state) => state.connection?.phase === "connected");
  const transcript = useDesktopStore((state) => state.transcript);
  const inProgress = isTurnInProgress(transcript);
  const pendingDraft = useDesktopStore((state) => state.composerDraft);

  // 업무 패널이 넘긴 글을 그 스레드가 열렸을 때 입력창에 채운다. 바로 보내지 않는다.
  useEffect(() => {
    if (pendingDraft && pendingDraft.threadId === transcript?.threadId) {
      setDraft(pendingDraft.text);
      useDesktopStore.getState().setComposerDraft(null);
    }
  }, [pendingDraft, transcript?.threadId]);
  const disabled = !connected || !transcript || sending;

  const run = async (action: () => Promise<void>) => {
    setError(null);
    setSending(true);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSending(false);
    }
  };

  const send = () => {
    const text = draft;
    if (text.trim().length === 0) {
      return;
    }
    void run(async () => {
      await actions.sendMessage(text);
      setDraft("");
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      send();
    }
  };

  return (
    <footer className="composer">
      {!connected && <p className="muted">연결이 끊겨 입력할 수 없습니다.</p>}
      {error && <p className="error">{error}</p>}
      <div className="composer-row">
        <textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={inProgress ? "진행 중인 턴에 추가 지시 (steer)" : "메시지 (Enter 보내기, Shift+Enter 줄바꿈)"}
          disabled={disabled}
          rows={3}
          data-testid="composer-input"
        />
        <div className="composer-buttons">
          <button type="button" onClick={send} disabled={disabled || draft.trim().length === 0} data-testid="composer-send">
            {inProgress ? "추가 지시" : "보내기"}
          </button>
          {inProgress && (
            <button type="button" onClick={() => void run(actions.interrupt)} disabled={!connected} data-testid="composer-interrupt">
              중단
            </button>
          )}
        </div>
      </div>
    </footer>
  );
}
