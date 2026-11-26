// 사이드바 스레드의 오른쪽 클릭 메뉴와 이름 바꾸기 칸이다 (#2841). 로컬 스레드에만 쓴다. 원격·dot 스레드는 읽기 전용이다.
import { type FormEvent, useEffect, useRef, useState } from "react";

import type { WorkspaceThread } from "../session/desktopStore";

/** 스레드에 대한 요청이다. App 이 열린 세션으로 보낸다. */
export interface ThreadActions {
  rename(threadId: string, name: string): Promise<void>;
  archive(threadId: string): Promise<void>;
  unarchive(threadId: string): Promise<void>;
  openFolder(path: string): Promise<boolean>;
}

export interface ThreadMenuProps {
  thread: WorkspaceThread;
  folder: string;
  running: boolean;
  position: { x: number; y: number };
  onRename(): void;
  onArchive(): void;
  onOpenFolder(): void;
  onClose(): void;
}

function copy(text: string): void {
  void navigator.clipboard.writeText(text).catch(() => undefined);
}

export function ThreadMenu({ thread, folder, running, position, onRename, onArchive, onOpenFolder, onClose }: ThreadMenuProps) {
  const menu = useRef<HTMLUListElement>(null);
  useEffect(() => {
    menu.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const close = (event: Event) => {
      if (event instanceof KeyboardEvent ? event.key === "Escape" : !menu.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", close);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", close);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);
  const item = (label: string, action: () => void, options: { disabled?: boolean; title?: string; testId?: string } = {}) => (
    <li>
      <button type="button" role="menuitem" disabled={options.disabled} title={options.title} data-testid={options.testId}
        onClick={() => { onClose(); action(); }}>{label}</button>
    </li>
  );
  return (
    <ul ref={menu} className="thread-menu" role="menu" aria-label="스레드 메뉴" style={{ left: position.x, top: position.y }} data-testid="thread-menu">
      {item("이름 바꾸기", onRename, { testId: "thread-menu-rename" })}
      {item("보관", onArchive, running
        ? { disabled: true, title: "진행 중인 턴이 있어 보관할 수 없습니다. 턴이 끝난 뒤에 하십시오.", testId: "thread-menu-archive" }
        : { testId: "thread-menu-archive" })}
      {item("폴더 열기", onOpenFolder, { testId: "thread-menu-open-folder" })}
      {item("경로 복사", () => copy(folder), { testId: "thread-menu-copy-path" })}
      {thread.branch && item("브랜치 복사", () => copy(thread.branch!), { testId: "thread-menu-copy-branch" })}
      {item("ID 복사", () => copy(thread.id), { testId: "thread-menu-copy-id" })}
    </ul>
  );
}

/** 이름 바꾸기 칸이다. Enter 로 보내고 Escape 나 포커스를 잃으면 취소한다. */
export function ThreadRenameInput({ title, onSubmit, onCancel }: { title: string; onSubmit(name: string): void; onCancel(): void }) {
  const [value, setValue] = useState(title);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (value.trim() && value.trim() !== title) onSubmit(value);
    else onCancel();
  };
  return (
    <form className="thread-rename" onSubmit={submit}>
      <input autoFocus aria-label="스레드 이름" value={value} maxLength={200} data-testid="thread-rename-input"
        onChange={(event) => setValue(event.target.value)} onBlur={onCancel}
        onKeyDown={(event) => { if (event.key === "Escape") onCancel(); }} onFocus={(event) => event.target.select()} />
    </form>
  );
}
