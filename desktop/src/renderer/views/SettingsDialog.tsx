// 설정 화면이다 (plan 26 S1). 글자 크기(Z4), 알림(A2), 잠자기 방지(W5), 화면 밝기와 확대를 한곳에서 바꾼다.
// 값은 main 이 저장하고 모든 창에 적용한다. 위의 입력란은 항목의 이름과 설명에서 찾는다.
import { type ReactNode, useEffect, useState } from "react";

import { type AppSettings, DEFAULT_SETTINGS, type FontArea, type KeepAwakeMode, MAX_FONT_PX, MIN_FONT_PX, type NotificationMode } from "../../shared/settings";
import { Icon } from "../design/icons";
import type { RepositoryExtensions } from "../extensions/repositoryExtensions";
import { extensionPanels, panelForExtension } from "../extensions/registry";
import { updateSettings, useSettings } from "../session/settingsStore";
import { matchesSetting } from "../session/settingsView";
import { MOD_KEY, openPanel, toggleTheme, useUiStore } from "../session/uiStore";

interface Row {
  id: string;
  section: string;
  label: string;
  hint: string;
  control: ReactNode;
}

function Segmented<T extends string>({ value, options, onChange, testId }: { value: T; options: readonly (readonly [T, string])[]; onChange(value: T): void; testId: string }) {
  return (
    <div className="segmented" data-testid={testId}>
      {options.map(([option, label]) => (
        <button key={option} type="button" className={option === value ? "selected" : undefined} aria-pressed={option === value} onClick={() => onChange(option)} data-value={option}>
          {label}
        </button>
      ))}
    </div>
  );
}

function Stepper({ value, label, onStep, onReset, canDecrease, canIncrease, testId }: { value: string; label: string; onStep(delta: -1 | 1): void; onReset(): void; canDecrease: boolean; canIncrease: boolean; testId: string }) {
  return (
    <div className="stepper" data-testid={testId}>
      <button type="button" className="icon-button" aria-label={`${label} 줄이기`} onClick={() => onStep(-1)} disabled={!canDecrease}>
        <Icon name="minus" size={14} />
      </button>
      <button type="button" className="stepper-value" title="기본값으로" onClick={onReset} data-testid={`${testId}-value`}>
        {value}
      </button>
      <button type="button" className="icon-button" aria-label={`${label} 늘리기`} onClick={() => onStep(1)} disabled={!canIncrease}>
        <Icon name="plus" size={14} />
      </button>
    </div>
  );
}

const FONT_ROWS: readonly (readonly [FontArea, string, string])[] = [
  ["ui", "UI 글자 크기", "사이드바, 단추, 패널의 글자"],
  ["reading", "대화 글자 크기", "내 메시지와 에이전트의 답"],
  ["code", "코드와 diff 글자 크기", "코드 블록, 명령 출력, 변경 보기"],
  ["terminal", "터미널 글자 크기", "아래 터미널의 글자"],
];

const NOTIFICATION_MODES: readonly (readonly [NotificationMode, string])[] = [
  ["off", "끄기"],
  ["background", "창이 뒤에 있을 때만"],
  ["always", "항상"],
];

const KEEP_AWAKE_MODES: readonly (readonly [KeepAwakeMode, string])[] = [
  ["off", "끄기"],
  ["working", "일할 때만"],
  ["on", "항상"],
];

function useZoomFactor(): number {
  const [factor, setFactor] = useState(1);
  useEffect(() => {
    window.akra.zoom.get().then(setFactor, () => undefined);
    return window.akra.zoom.onChanged(setFactor);
  }, []);
  return factor;
}

function settingsRows(settings: AppSettings, theme: string, zoom: number): Row[] {
  const fontRows = FONT_ROWS.map<Row>(([area, label, hint]) => {
    const size = settings.fonts[area];
    const setSize = (next: number) => updateSettings({ fonts: { [area]: next } });
    return {
      id: `font-${area}`,
      section: "글자",
      label,
      hint,
      control: <Stepper value={`${size}px`} label={label} onStep={(delta) => setSize(size + delta)} onReset={() => setSize(DEFAULT_SETTINGS.fonts[area])} canDecrease={size > MIN_FONT_PX} canIncrease={size < MAX_FONT_PX} testId={`settings-font-${area}`} />,
    };
  });
  return [
    {
      id: "theme",
      section: "화면",
      label: "밝기",
      hint: "밝은 화면 또는 어두운 화면",
      control: (
        <Segmented
          value={theme}
          options={[
            ["light", "밝게"],
            ["dark", "어둡게"],
          ]}
          onChange={(next) => next !== theme && toggleTheme()}
          testId="settings-theme"
        />
      ),
    },
    {
      id: "zoom",
      section: "화면",
      label: "확대",
      hint: `앱 전체의 배율 (${MOD_KEY}+ · ${MOD_KEY}− · ${MOD_KEY}0)`,
      control: <Stepper value={`${Math.round(zoom * 100)}%`} label="확대" onStep={(delta) => window.akra.zoom.step(delta === 1 ? "in" : "out")} onReset={() => window.akra.zoom.step("reset")} canDecrease={zoom > 0.5} canIncrease={zoom < 2} testId="settings-zoom" />,
    },
    ...fontRows,
    {
      id: "notify-mode",
      section: "알림",
      label: "운영체제 알림",
      hint: "턴 완료, 실패, 승인 요청, dot 의 답을 알린다",
      control: <Segmented value={settings.notifications.mode} options={NOTIFICATION_MODES} onChange={(mode) => updateSettings({ notifications: { mode } })} testId="settings-notify-mode" />,
    },
    {
      id: "notify-sound",
      section: "알림",
      label: "알림 소리",
      hint: "알림을 보일 때 소리를 낸다",
      control: (
        <input type="checkbox" checked={settings.notifications.sound} onChange={(event) => updateSettings({ notifications: { sound: event.target.checked } })} aria-label="알림 소리" data-testid="settings-notify-sound" />
      ),
    },
    {
      id: "keep-awake",
      section: "전원",
      label: "잠자기 방지",
      hint: "에이전트가 일하는 동안 컴퓨터가 잠들지 않게 한다. 화면은 꺼질 수 있다",
      control: <Segmented value={settings.keepAwake} options={KEEP_AWAKE_MODES} onChange={(keepAwake) => updateSettings({ keepAwake })} testId="settings-keep-awake" />,
    },
  ];
}

function ExtensionControl({ extensions, id, enabled, compatible, lifecycle }: { extensions: RepositoryExtensions; id: string; enabled: boolean; compatible: boolean; lifecycle: string }) {
  const [error, setError] = useState("");
  const panel = panelForExtension(id);
  return <div>
    <div className="segmented">
      <button disabled={!extensions.ready || (!compatible && !enabled) || extensions.pending.includes(id) || lifecycle === "disabling"} aria-pressed={enabled} onClick={async () => {
        setError("");
        try { await extensions.configure(id, !enabled); } catch (failure) { setError(String(failure)); }
      }}>{extensions.pending.includes(id) ? "상태 변경 중…" : lifecycle === "disabling" ? "끄는 중…" : enabled ? "끄기" : "켜기"}</button>
      {enabled && lifecycle === "enabled" && extensions.enabledIds.includes(id) && panel && <button onClick={() => openPanel(panel.id)}>열기</button>}
    </div>
    {error && <p role="alert">{error}</p>}
  </div>;
}

export function SettingsDialog({ extensions, localExtensions, chooseFolder }: { extensions?: RepositoryExtensions; localExtensions?: RepositoryExtensions; chooseFolder?: () => Promise<void> } = {}) {
  const settings = useSettings();
  const theme = useUiStore((state) => state.theme);
  const zoom = useZoomFactor();
  const [query, setQuery] = useState("");
  const close = () => useUiStore.setState({ overlay: null });
  const entries = settingsRows(settings, theme, zoom);
  for (const [scope, source] of [["local_user", localExtensions], ["repository", extensions]] as const) {
    if (!source) continue;
    const items = source.items.length ? source.items : extensionPanels.filter(panel => panel.scope === scope).map(panel => ({
      ...panel, enabled: false, desired_enabled: false, compatible: true, lifecycle: "disabled", compatibility_reason: null, error: null, revision: 0, generation: 0,
    }));
    for (const item of items) {
      const panel = panelForExtension(item.id);
      const scopeName = scope === "local_user" ? "이 PC · 현재 사용자" : "현재 저장소";
      const status = item.lifecycle === "disabling" ? "끄는 중" : item.lifecycle === "error" ? "전환 오류" : item.enabled ? "켜짐" : "꺼짐";
      const reason = source.error || item.error || item.compatibility_reason;
      const description = item.id === "akra.tech-blog" ? " · 이 저장소의 작업으로 기술 블로그 글을 자동으로 만듭니다. 모델 사용량을 씁니다." : "";
      entries.push({ id: item.id, section: "확장", label: panel?.label ?? item.id,
        hint: `${scopeName} · 버전 ${item.version} · ${status}${description}${reason ? ` · ${reason}` : ""}${scope === "local_user" ? " · 변경하면 같은 사용자의 다른 창에도 적용됩니다." : ""}`,
        control: source.kernel
          ? <ExtensionControl key={`${source.kernel}:${item.id}`} extensions={source} id={item.id} enabled={item.enabled} compatible={item.compatible} lifecycle={item.lifecycle} />
          : scope === "repository" ? <button onClick={() => void chooseFolder?.()}>저장소 선택</button>
          : <span role="status">{source.error ? "로컬 호스트 연결 실패" : "로컬 호스트 연결 중…"}</span>,
      });
    }
  }
  const rows = entries.filter((row) => matchesSetting(row, query));
  const sections = [...new Set(rows.map((row) => row.section))];
  return (
    <div className="overlay" onClick={close}>
      <div className="dialog settings" role="dialog" aria-label="설정" onClick={(event) => event.stopPropagation()} data-testid="settings">
        <header>
          <strong>설정</strong>
          <button type="button" className="icon-button" title="닫기" onClick={close}>
            <Icon name="close" />
          </button>
        </header>
        <div className="palette-input">
          <Icon name="search" />
          <input autoFocus value={query} placeholder="설정 찾기" onChange={(event) => setQuery(event.target.value)} data-testid="settings-search" />
        </div>
        <div className="settings-body">
          {sections.map((section) => (
            <section key={section}>
              <h4>{section}</h4>
              {rows
                .filter((row) => row.section === section)
                .map((row) => (
                  <div key={row.id} className="settings-row" data-testid={`settings-row-${row.id}`}>
                    <div className="settings-text">
                      <span>{row.label}</span>
                      <span className="muted">{row.hint}</span>
                    </div>
                    {row.control}
                  </div>
                ))}
            </section>
          ))}
          {rows.length === 0 && <p className="empty-row">찾는 설정이 없습니다</p>}
        </div>
      </div>
    </div>
  );
}
