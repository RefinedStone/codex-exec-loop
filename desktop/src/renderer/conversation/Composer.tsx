// 입력창 (P3 §6.2). 위에는 승인 요청과 계획 진행을 붙인다 (R10). Enter 는 보내기, Shift+Enter 는 줄바꿈이다.
// 턴이 진행 중이면 보내기는 steer 가 되고 중단 버튼이 보인다. 연결이 끊기면 입력을 막는다 (§6.5).
// `home` 이면 새 스레드 화면의 입력창이다. 보내면 새 스레드를 만들고 첫 메시지를 보낸다.
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";

import type { TurnPlanStep } from "../../shared/protocol/generated/v2/TurnPlanStep";
import { Icon } from "../design/icons";
import { useDesktopStore } from "../session/desktopStore";
import { HOME_DRAFT, setDraft as setDraftFor, useUiStore } from "../session/uiStore";
import { isSendKey, recallCursor } from "./composerKeys";
import { type ActiveMention, activeMention, applyMention, sameMention } from "./fileMention";
import { type ImageAttachment, readImage, refuseAttachment, withoutSent } from "./imageAttachments";
import { isSentComposerDraft, runComposerAction, useComposerRequests } from "./composerAction";
import { PlanList } from "./ConversationView";
import { loadPrompts, rememberPrompt } from "./promptHistory";
import { contextPercent, isTurnInProgress } from "./transcript";
import type { NextTurnQueue } from "../session/nextTurnQueue";
import { QueuedInputs } from "./QueuedInputs";
import type { NextTurnSettingsController } from "../session/nextTurnSettings";
import { NextTurnSettings } from "./NextTurnSettings";

export interface ComposerActions {
  /** 대화 폴더의 파일을 찾는다 (#2837). 없으면 `@` 후보를 보이지 않는다. */
  searchFiles?(query: string): Promise<string[]>;
  modelSettings?: NextTurnSettingsController;
  queue?: NextTurnQueue;
  queueMessage?(text: string): void;
  resumeQueuedMessages?(): void;
  /** `images` 는 첨부한 이미지의 data URL 이다 (#2838). */
  sendMessage(text: string, images?: readonly string[]): Promise<void>;
  interrupt(): Promise<void>;
  /** 새 스레드를 만들고 첫 메시지를 보낸다. 사용자가 폴더 고르기를 취소하면 false 이다. */
  startThread(text: string): Promise<boolean>;
}

const MAX_INPUT_HEIGHT_PX = 220;
/** 글쇠를 누를 때마다 찾지 않게 잠깐 기다린다. */
const MENTION_SEARCH_DELAY_MS = 120;
/** 이 안에서 누른 글자 키는 그 칸의 것이다. 입력창으로 옮기지 않는다. */
const TYPING_TARGETS = "input, textarea, select, button, a[href], [contenteditable], .terminal-body, .right-panel, .overlay, dialog";

function PlanStrip({ steps }: { steps: readonly TurnPlanStep[] }) {
  const [open, setOpen] = useState(false);
  const done = steps.filter((step) => step.status === "completed").length;
  const active = steps.find((step) => step.status === "inProgress");
  return (
    <div className="plan-strip">
      <button type="button" className="plan-strip-line" onClick={() => setOpen(!open)} aria-expanded={open}>
        <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
        <span className="plan-progress">
          계획 {done}/{steps.length}
        </span>
        <span className="plan-bar">
          <span style={{ width: `${(done / steps.length) * 100}%` }} />
        </span>
        {active && <span className="plan-active">{active.step}</span>}
      </button>
      {open && <PlanList steps={steps} />}
    </div>
  );
}

function ContextRing({ percent }: { percent: number }) {
  const circumference = 2 * Math.PI * 7;
  return (
    <span className="context-ring" title={`컨텍스트 ${percent}% 사용`} data-testid="context-ring">
      <svg width="18" height="18" viewBox="0 0 18 18">
        <circle cx="9" cy="9" r="7" fill="none" stroke="var(--line-strong)" strokeWidth="2" />
        <circle cx="9" cy="9" r="7" fill="none" stroke="var(--accent)" strokeWidth="2" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - percent / 100)} transform="rotate(-90 9 9)" strokeLinecap="round" />
      </svg>
      {percent}%
    </span>
  );
}

export function Composer({ actions, home = false, above }: { actions: ComposerActions; home?: boolean; above?: ReactNode }) {
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const connected = useDesktopStore((state) => state.connection?.phase === "connected");
  const transcript = useDesktopStore((state) => (home ? null : state.transcript));
  // 다른 Codex 앱이 쓰기 잠금을 가진 스레드는 읽기만 한다. 보내기는 위의 배너에서 다시 연결한 뒤에 된다.
  const writerElsewhere = useDesktopStore((state) => !home && state.writerElsewhere);
  const inProgress = isTurnInProgress(transcript);
  const livePlan = inProgress ? (transcript?.turns.at(-1)?.plan ?? null) : null;
  const percent = contextPercent(transcript);
  // 초안은 화면 저장소에 둔다. 보기를 바꾸거나 업무 패널이 글을 넘겨도 같은 값을 본다.
  const draftKey = home ? HOME_DRAFT : (transcript?.threadId ?? "");
  const draft = useUiStore((state) => state.drafts[draftKey] ?? "");
  const sending = useComposerRequests((state) => state.pendingKeys.includes(draftKey));
  const setDraft = (text: string) => setDraftFor(draftKey, text);
  const disabled = !connected || (!home && !transcript) || writerElsewhere || sending;

  useEffect(() => {
    if (!disabled) input.current?.focus();
  }, [transcript?.threadId, home, disabled]);

  // 대화 영역에서 글자를 치면 입력창으로 옮긴다 (#2839). 입력 칸, 단추, 터미널, 패널, 대화 상자의 키는 그대로 둔다.
  useEffect(() => {
    if (disabled) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey || event.isComposing) return;
      if (event.key.length !== 1 || event.key.trim().length === 0) return;
      const target = event.target;
      if (target instanceof Element && target.closest(TYPING_TARGETS)) return;
      input.current?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [disabled]);

  // 내용에 맞춰 입력창 높이를 늘린다.
  useEffect(() => {
    const element = input.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_INPUT_HEIGHT_PX)}px`;
  }, [draft]);

  const run = async (action: () => Promise<void>) => {
    setError(null);
    try {
      await runComposerAction(draftKey, action);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  // 이미지 첨부이다 (#2838). 입력창은 스레드마다 새로 만들므로 첨부는 다른 스레드로 따라가지 않는다.
  const [attachments, setAttachments] = useState<ImageAttachment[]>([]);
  // 읽는 중인 파일도 첨부 자리를 차지한다. 읽기가 끝나기 전에 또 붙여도 같은 자리를 두 번 세지 않는다.
  // shortcut: 읽기가 끝난 뒤 다시 그리기 전의 짧은 틈에 붙이면 MAX_IMAGES 를 하나 넘길 수 있다 (버리지는 않는다). 문제가 되면 reducer 로 옮긴다.
  const pendingReads = useRef(0);
  const attachFiles = (files: readonly File[]) => {
    for (const file of files) {
      const refused = refuseAttachment(file, attachments.length + pendingReads.current);
      if (refused) {
        setError(refused);
        continue;
      }
      pendingReads.current += 1;
      readImage(file).then(
        (image) => setAttachments((current) => [...current, image]),
        (cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)),
      ).finally(() => {
        pendingReads.current -= 1;
      });
    }
  };
  const attachRef = useRef(attachFiles);
  attachRef.current = attachFiles;
  // 대화 창 어디에 놓아도 이 입력창에 붙는다. 터미널과 오른쪽 패널은 제외한다.
  useEffect(() => {
    if (home) return;
    const accepts = (event: DragEvent) => event.dataTransfer?.types.includes("Files")
      && !(event.target instanceof Element && event.target.closest(".terminal-body, .right-panel"));
    const over = (event: DragEvent) => {
      if (!accepts(event)) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    };
    const drop = (event: DragEvent) => {
      if (!accepts(event)) return;
      event.preventDefault();
      attachRef.current([...(event.dataTransfer?.files ?? [])]);
    };
    window.addEventListener("dragover", over);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("dragover", over);
      window.removeEventListener("drop", drop);
    };
  }, [home]);

  const send = () => {
    const text = draft;
    const sent = attachments;
    const images = sent.map((image) => image.url);
    if ((text.trim().length === 0 && images.length === 0) || disabled) {
      return;
    }
    void run(async () => {
      if (home) {
        if (!(await actions.startThread(text))) return;
      } else {
        await actions.sendMessage(text, images);
        setAttachments((current) => withoutSent(current, sent));
      }
      rememberPrompt(text);
      if (isSentComposerDraft(useUiStore.getState().drafts[draftKey], text)) clearDraft();
    });
  };

  /**
   * ↑ / ↓ 로 불러온 이전 프롬프트이다 (plan 26 I1). 입력창이 비었거나 불러온 글이 그대로일 때만 옮긴다: 사용자가 글을
   * 고치면 화살표는 다시 커서를 옮긴다.
   */
  const recalled = useRef<{ cursor: number; text: string } | null>(null);
  const recall = (direction: "older" | "newer"): boolean => {
    const browsing = recalled.current !== null && recalled.current.text === draft;
    if (!browsing && (draft !== "" || direction === "newer")) return false;
    const prompts = loadPrompts();
    const cursor = recallCursor(prompts.length, browsing ? (recalled.current?.cursor ?? null) : null, direction);
    const text = cursor === null ? "" : (prompts[cursor] ?? "");
    recalled.current = cursor === null ? null : { cursor, text };
    closeMention();
    setDraft(text);
    return true;
  };

  // `@` 파일 언급이다 (#2837). 입력이 바뀔 때마다 번호를 올려서 늦게 온 결과(이전 글, 다른 대화)를 버린다.
  // 입력창은 스레드마다 새로 만들므로, 떠난 스레드의 결과는 이 번호로 막힌다.
  const [mention, setMention] = useState<{ at: ActiveMention; files: string[]; cursor: number } | null>(null);
  const mentionSeq = useRef(0);
  // 마지막으로 찾기 시작한 언급이다. 커서만 움직여 언급이 바뀌면 (onSelect) 다시 찾거나 닫는다.
  const mentionAt = useRef<ActiveMention | null>(null);
  useEffect(() => () => {
    mentionSeq.current += 1;
  }, []);
  const closeMention = () => {
    mentionSeq.current += 1;
    mentionAt.current = null;
    setMention(null);
  };
  /** 초안을 코드로 비운다. 열린 후보와 진행 중인 찾기도 버린다. */
  const clearDraft = () => {
    closeMention();
    setDraft("");
  };
  const updateMention = (text: string, caret: number) => {
    const seq = ++mentionSeq.current;
    const at = actions.searchFiles ? activeMention(text, caret) : null;
    mentionAt.current = at;
    if (!at || !actions.searchFiles) {
      setMention(null);
      return;
    }
    const search = actions.searchFiles;
    window.setTimeout(() => {
      if (mentionSeq.current !== seq) return;
      search(at.query).then(
        (files) => { if (mentionSeq.current === seq) setMention(files.length > 0 ? { at, files, cursor: 0 } : null); },
        () => { if (mentionSeq.current === seq) setMention(null); },
      );
    }, MENTION_SEARCH_DELAY_MS);
  };
  const chooseMention = (path: string) => {
    if (!mention) return;
    const caret = input.current?.selectionStart ?? draft.length;
    // 후보를 연 뒤 커서가 언급 밖으로 나갔으면 바꾸지 않는다. 바꾸면 커서까지의 글이 지워진다.
    const at = activeMention(draft, caret);
    closeMention();
    if (!at || at.start !== mention.at.start) return;
    const next = applyMention(draft, at, caret, path);
    setDraft(next.text);
    requestAnimationFrame(() => input.current?.setSelectionRange(next.caret, next.caret));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (mention && !event.nativeEvent.isComposing) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : -1;
        setMention({ ...mention, cursor: (mention.cursor + step + mention.files.length) % mention.files.length });
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        chooseMention(mention.files[mention.cursor]!);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        closeMention();
        return;
      }
    }
    if (isSendKey(event)) {
      event.preventDefault();
      send();
      return;
    }
    const plain = !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey && !event.nativeEvent.isComposing;
    if (plain && (event.key === "ArrowUp" || event.key === "ArrowDown") && recall(event.key === "ArrowUp" ? "older" : "newer")) {
      event.preventDefault();
    }
  };

  const sendTitle = inProgress ? "추가 지시" : "보내기";
  const reserve = () => {
    if (disabled || !draft.trim() || !actions.queueMessage) return;
    try {
      actions.queueMessage(draft);
      clearDraft();
      setError(null);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  return (
    <div className="composer-area">
      <div className="column">
        {above}
        {!home && transcript && actions.queue && actions.resumeQueuedMessages && <QueuedInputs queue={actions.queue} threadId={transcript.threadId} resume={actions.resumeQueuedMessages} />}
        {livePlan && livePlan.length > 0 && <PlanStrip steps={livePlan} />}
        {!connected && <p className="muted">연결이 끊겨 입력할 수 없습니다.</p>}
        {error && <p className="error">{error}</p>}
        {!home && transcript && actions.modelSettings && <NextTurnSettings key={transcript.threadId}
          controller={actions.modelSettings} threadId={transcript.threadId} disabled={!connected || writerElsewhere} />}
        <div className="composer">
          {mention && (
            <ul className="mention-list" role="listbox" aria-label="파일 언급" data-testid="mention-list">
              {mention.files.map((path, index) => (
                <li key={path} role="option" aria-selected={index === mention.cursor}>
                  <button type="button" className={index === mention.cursor ? "selected" : undefined}
                    onMouseDown={(event) => event.preventDefault()} onClick={() => chooseMention(path)}>{path}</button>
                </li>
              ))}
            </ul>
          )}
          {attachments.length > 0 && (
            <ul className="attachments" data-testid="attachments">
              {attachments.map((image) => (
                <li key={image.id} className="attachment">
                  <img src={image.url} alt={image.name} />
                  <button type="button" className="icon-button" title={`${image.name} 빼기`} aria-label={`${image.name} 빼기`}
                    onClick={() => setAttachments((current) => current.filter((item) => item.id !== image.id))} data-testid="attachment-remove">
                    <Icon name="close" size={12} />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <textarea
            ref={input}
            value={draft}
            rows={1}
            onChange={(event) => {
              setDraft(event.target.value);
              updateMention(event.target.value, event.target.selectionStart);
            }}
            onSelect={(event) => {
              const { value, selectionStart } = event.currentTarget;
              if (!sameMention(activeMention(value, selectionStart), mentionAt.current)) updateMention(value, selectionStart);
            }}
            onKeyDown={onKeyDown}
            onPaste={home ? undefined : (event) => {
              const images = [...event.clipboardData.files].filter((file) => file.type.startsWith("image/"));
              if (images.length === 0) return;
              event.preventDefault();
              attachFiles(images);
            }}
            placeholder={writerElsewhere ? "다른 Codex 앱이 이 스레드를 쓰는 동안 보낼 수 없습니다" : inProgress ? "진행 중인 턴에 추가 지시 (steer)" : "무엇을 할까요?"}
            disabled={disabled}
            data-testid="composer-input"
          />
          <div className="composer-bar">
            {!home && actions.queueMessage && <button type="button" className="subtle-button" disabled={disabled || !draft.trim()} onClick={reserve} data-testid="composer-queue">다음 턴 예약</button>}
            <span className="bar-spacer" />
            {percent !== null && <ContextRing percent={percent} />}
            {inProgress && (
              <button type="button" className="round-button stop" title="중단" onClick={() => void run(actions.interrupt)} disabled={!connected || sending} data-testid="composer-interrupt">
                <span className="stop-square" />
              </button>
            )}
            <button type="button" className="round-button send" title={sendTitle} aria-label={sendTitle} onClick={send} disabled={disabled || (draft.trim().length === 0 && attachments.length === 0)} data-testid="composer-send">
              <Icon name="arrowUp" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
