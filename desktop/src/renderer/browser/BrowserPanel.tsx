// 화면 오른쪽의 브라우저 패널이다 (R3). 도구 막대(프로필, 프리셋, 주소창)는 여기서 그리고, 페이지는 main 의
// WebContentsView 가 아래 빈 자리 위에 그린다. 그래서 이 자리의 위치와 크기를 바뀔 때마다 main 에 알린다.
import { type FormEvent, useEffect, useLayoutEffect, useRef, useState } from "react";

import type { BrowserBridge, BrowserSettings, BrowserState } from "../../shared/bridge";
import type { BrowserAgentState } from "../../shared/browserAgent";
import { profileIdFromName } from "./profileId";

type Editing = { kind: "profile" | "preset"; value: string } | null;


export function BrowserPanel({ bridge, threadId, onStartConversation }: {
  bridge: BrowserBridge;
  threadId: string | null;
  onStartConversation(): Promise<void>;
}) {
  const slotRef = useRef<HTMLDivElement>(null);
  const [settings, setSettings] = useState<BrowserSettings | null>(null);
  const [profileId, setProfileId] = useState("default");
  const [state, setState] = useState<BrowserState | null>(null);
  const [address, setAddress] = useState("");
  const [editingAddress, setEditingAddress] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing>(null);
  const [agent, setAgent] = useState<BrowserAgentState | null>(null);
  const [agentBusy, setAgentBusy] = useState(false);

  useEffect(() => {
    let current = true;
    let received = false;
    const unsubscribe = bridge.agent.onState((next) => {
      received = true;
      if (current) setAgent(next);
    });
    void bridge.agent.status().then((next) => {
      if (current && !received) setAgent(next);
    }, (error: unknown) => {
      if (current) setMessage(error instanceof Error ? error.message : String(error));
    });
    return () => {
      current = false;
      unsubscribe();
    };
  }, [bridge, threadId]);

  const agentAction = async (action: () => Promise<void>) => {
    setAgentBusy(true);
    setMessage(null);
    try {
      await action();
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setAgentBusy(false);
    }
  };
  const matchingAgent = threadId !== null && agent?.threadId === threadId;
  const canPause = matchingAgent && (agent?.phase === "active" || agent?.phase === "handoff");
  const canResume = matchingAgent && (agent?.phase === "paused" || agent?.phase === "handoff");
  const agentLabel = !agent ? "브라우저 연결 확인 중" : !matchingAgent && threadId
    ? "이 대화에서 브라우저 제어를 사용할 수 없습니다."
    : ({ idle: "직접 제어", active: "에이전트 제어", paused: "일시 정지", handoff: "사용자 입력 대기", unavailable: "브라우저 제어 사용 불가" } as const)[agent.phase];

  const bounds = () => {
    const rect = slotRef.current?.getBoundingClientRect();
    return rect ? { x: rect.left, y: rect.top, width: rect.width, height: rect.height } : { x: 0, y: 0, width: 0, height: 0 };
  };

  useEffect(() => {
    void bridge.loadSettings().then((loaded) => {
      setSettings(loaded);
      setProfileId(loaded.profiles[0]?.id ?? "default");
    });
    const unsubscribe = bridge.onState((next) => {
      setState(next);
      if (next.error) setMessage(`페이지를 열 수 없습니다: ${next.error}`);
    });
    return () => {
      unsubscribe();
      bridge.hide();
    };
  }, [bridge]);

  // 프로필이 정해지면(또는 바뀌면) 그 프로필의 세션으로 보기를 연다.
  useLayoutEffect(() => {
    if (!settings) return;
    void bridge.show(profileId, bounds(), null).then(setState);
  }, [bridge, settings, profileId]);

  // 자리의 크기나 위치가 바뀌면 보기도 옮긴다.
  useEffect(() => {
    const slot = slotRef.current;
    if (!slot) return;
    const update = () => bridge.setBounds(bounds());
    const observer = new ResizeObserver(update);
    observer.observe(slot);
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [bridge]);

  useEffect(() => {
    if (!editingAddress) setAddress(state?.url && state.url !== "about:blank" ? state.url : "");
  }, [state?.url, editingAddress]);

  const open = async (url: string, targetProfile = profileId) => {
    setMessage(null);
    if (targetProfile !== profileId) {
      setProfileId(targetProfile);
      setState(await bridge.show(targetProfile, bounds(), url));
      return;
    }
    if (!(await bridge.navigate(url))) setMessage("http(s) 주소만 열 수 있습니다.");
  };

  const submitAddress = (event: FormEvent) => {
    event.preventDefault();
    setEditingAddress(false);
    void open(address);
  };

  const save = async (next: BrowserSettings) => setSettings(await bridge.saveSettings(next));

  const submitEditing = (event: FormEvent) => {
    event.preventDefault();
    if (!settings || !editing || !editing.value.trim()) return setEditing(null);
    const name = editing.value.trim();
    if (editing.kind === "profile") {
      const id = profileIdFromName(name, settings.profiles.map((profile) => profile.id));
      void save({ ...settings, profiles: [...settings.profiles, { id, name }] }).then(() => setProfileId(id));
    } else if (state?.url && state.url !== "about:blank") {
      void save({ ...settings, presets: [...settings.presets.filter((preset) => preset.name !== name), { name, url: state.url, profileId }] });
    }
    setEditing(null);
  };

  return (
    <aside className="browser-panel" data-testid="browser-panel">
      <div className="browser-toolbar">
        <select value={profileId} onChange={(event) => setProfileId(event.target.value)} title="프로필 (쿠키와 로그인이 따로 저장됩니다)" data-testid="browser-profile">
          {settings?.profiles.map((profile) => (
            <option key={profile.id} value={profile.id}>
              {profile.name}
            </option>
          ))}
        </select>
        <button type="button" onClick={() => setEditing({ kind: "profile", value: "" })} title="프로필 추가" data-testid="browser-add-profile">
          +프로필
        </button>
        <select
          value=""
          onChange={(event) => {
            const preset = settings?.presets.find((item) => item.name === event.target.value);
            if (preset) void open(preset.url, preset.profileId);
          }}
          data-testid="browser-preset"
        >
          <option value="">프리셋…</option>
          {settings?.presets.map((preset) => (
            <option key={preset.name} value={preset.name}>
              {preset.name}
            </option>
          ))}
        </select>
        <button type="button" onClick={() => setEditing({ kind: "preset", value: state?.title ?? "" })} disabled={!state?.url || state.url === "about:blank"} data-testid="browser-save-preset">
          프리셋 저장
        </button>
      </div>
      {editing && (
        <form className="browser-toolbar" onSubmit={submitEditing}>
          <input
            autoFocus
            value={editing.value}
            onChange={(event) => setEditing({ ...editing, value: event.target.value })}
            placeholder={editing.kind === "profile" ? "새 프로필 이름" : "프리셋 이름"}
            data-testid="browser-editing-input"
          />
          <button type="submit">저장</button>
          <button type="button" onClick={() => setEditing(null)}>
            취소
          </button>
        </form>
      )}
      <form className="browser-toolbar" onSubmit={submitAddress}>
        <button type="button" onClick={() => bridge.command("back")} disabled={!state?.canGoBack} aria-label="뒤로">
          ←
        </button>
        <button type="button" onClick={() => bridge.command("forward")} disabled={!state?.canGoForward} aria-label="앞으로">
          →
        </button>
        <button type="button" onClick={() => bridge.command(state?.loading ? "stop" : "reload")} aria-label={state?.loading ? "멈춤" : "새로 고침"}>
          {state?.loading ? "×" : "↻"}
        </button>
        <input
          className="browser-address"
          value={address}
          onChange={(event) => {
            setEditingAddress(true);
            setAddress(event.target.value);
          }}
          onBlur={() => setEditingAddress(false)}
          placeholder="주소 (예: qa.example.com)"
          data-testid="browser-address"
        />
      </form>
      <div className="browser-agent" aria-busy={agentBusy}>
        <div className="browser-toolbar">
          <span className="browser-agent-status" role="status" data-testid="browser-agent-status">{agentLabel}</span>
          {canPause && <button type="button" disabled={agentBusy} onClick={() => void agentAction(() => bridge.agent.pause())} data-testid="browser-agent-pause">일시 정지</button>}
          {canResume
            ? <button type="button" disabled={agentBusy} onClick={() => void agentAction(() => bridge.agent.resume())} data-testid="browser-agent-resume">제어 재개</button>
            : !canPause && <button type="button" disabled={agentBusy || !settings || !agent} onClick={() => void agentAction(onStartConversation)} data-testid="browser-agent-new">새 브라우저 대화</button>}
        </div>
        {(matchingAgent || agent?.phase === "unavailable") && (agent?.reason || agent?.lastAction) && <div className="browser-message browser-agent-detail">
          {agent.reason && <div>{agent.reason}</div>}
          {agent.lastAction && <div>마지막 동작: {agent.lastAction}</div>}
        </div>}
      </div>
      {message && (
        <div className="error browser-message" data-testid="browser-message">
          {message}
        </div>
      )}
      <div className="browser-slot" ref={slotRef} data-testid="browser-slot" title={state?.title ?? ""} />
    </aside>
  );
}
