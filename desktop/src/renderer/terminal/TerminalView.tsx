// 터미널 한 개이다 (R2). xterm.js 로 그리고, main 의 셸과 IPC 로 주고받는다. 탭을 바꿔도 지우지 않고 숨기기만 한다.
import "@xterm/xterm/css/xterm.css";

import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";

import type { TerminalBridge } from "../../shared/bridge";

export interface TerminalViewProps {
  bridge: TerminalBridge;
  cwd: string | null;
  visible: boolean;
  onOpened(info: { cwd: string; shell: string }): void;
  onExited(exitCode: number): void;
}

/** 시험이 화면 대신 버퍼를 읽을 수 있게 터미널을 요소에 붙여 둔다 (WebGL 로 그리면 DOM 에 글자가 없다). */
export interface TerminalElement extends HTMLDivElement {
  akraTerminal?: Terminal;
}

export function TerminalView({ bridge, cwd, visible, onOpened, onExited }: TerminalViewProps) {
  const containerRef = useRef<TerminalElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<number | null>(null);
  const callbacks = useRef({ onOpened, onExited });
  callbacks.current = { onOpened, onExited };

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const terminal = new Terminal({
      fontFamily: 'ui-monospace, "Cascadia Mono", Consolas, monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 10_000,
      allowProposedApi: false,
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(container);
    container.akraTerminal = terminal;
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      terminal.loadAddon(webgl);
    } catch {
      // WebGL 을 쓸 수 없으면 기본 DOM 렌더러로 그린다.
    }
    fitRef.current = fit;
    fit.fit();

    let disposed = false;
    const unsubscribeData = bridge.onData((id, data) => {
      if (id === idRef.current) terminal.write(data);
    });
    const unsubscribeExit = bridge.onExit((id, exitCode) => {
      if (id !== idRef.current) return;
      terminal.write(`\r\n[셸이 끝났습니다: 종료 코드 ${exitCode}]\r\n`);
      idRef.current = null;
      callbacks.current.onExited(exitCode);
    });
    const input = terminal.onData((data) => {
      if (idRef.current !== null) bridge.write(idRef.current, data);
    });
    const resize = terminal.onResize(({ cols, rows }) => {
      if (idRef.current !== null) bridge.resize(idRef.current, cols, rows);
    });

    void bridge.open({ cwd, cols: terminal.cols, rows: terminal.rows }).then((result) => {
      if (disposed) {
        if (result.ok) bridge.close(result.id);
        return;
      }
      if (!result.ok) {
        terminal.write(`셸을 열 수 없습니다: ${result.message}\r\n`);
        return;
      }
      idRef.current = result.id;
      callbacks.current.onOpened({ cwd: result.cwd, shell: result.shell });
    });

    const observer = new ResizeObserver(() => {
      if (container.offsetParent !== null) fit.fit();
    });
    observer.observe(container);

    return () => {
      disposed = true;
      observer.disconnect();
      unsubscribeData();
      unsubscribeExit();
      input.dispose();
      resize.dispose();
      if (idRef.current !== null) bridge.close(idRef.current);
      idRef.current = null;
      terminal.dispose();
    };
  }, [bridge]); // 터미널은 처음 연 cwd 로 계속 쓴다. cwd 가 바뀌어도 다시 열지 않는다.

  // 숨겨 둔 탭을 다시 보이면 크기를 다시 맞추고 입력을 받게 한다.
  useEffect(() => {
    if (visible) {
      fitRef.current?.fit();
      containerRef.current?.akraTerminal?.focus();
    }
  }, [visible]);

  return <div className="terminal-view" ref={containerRef} data-testid="terminal-view" hidden={!visible} />;
}
