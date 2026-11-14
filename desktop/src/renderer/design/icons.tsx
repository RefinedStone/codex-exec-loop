// 블루프린트의 선 아이콘이다. 의존성을 더하지 않으려고 필요한 것만 24x24 path 로 둔다.
const PATHS = {
  plus: "M12 5v14M5 12h14",
  minus: "M5 12h14",
  undo: "M9 14L4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3",
  search: "M11 4a7 7 0 1 0 0 14a7 7 0 0 0 0-14zM20.5 20.5l-4.6-4.6",
  sidebar: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM9 4v16",
  panel: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM15 4v16",
  terminal: "M5 8l4 4-4 4M12 16h7",
  chevronRight: "M9 6l6 6-6 6",
  chevronDown: "M6 9l6 6 6-6",
  check: "M5 12.5l4.5 4.5L19 7.5",
  close: "M6 6l12 12M18 6L6 18",
  arrowUp: "M12 19V5M5.5 11.5L12 5l6.5 6.5",
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  branch: "M6 8.5v7M6 8.5a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM6 19.5a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM18 10.5a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM18 10.5c0 4-5 3.5-9.5 5.5",
  globe: "M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18zM3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18",
  file: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5",
  diff: "M12 4v8M8 8h8M8 18h8",
  grid: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
  bolt: "M13 2.5L4.5 14H11l-1 7.5L18.5 10H12z",
  briefcase: "M5 7h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2zM9 7V5.5A1.5 1.5 0 0 1 10.5 4h3A1.5 1.5 0 0 1 15 5.5V7",
  paperclip: "M20 11.5l-8.2 8.2a5 5 0 0 1-7-7l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7l-8.4 8.4a1.7 1.7 0 0 1-2.4-2.4l7.6-7.6",
  at: "M12 8.5a3.5 3.5 0 1 0 0 7a3.5 3.5 0 0 0 0-7zM15.5 8.5v5a2.5 2.5 0 0 0 5 0V12a8.5 8.5 0 1 0-3.4 6.8",
  sun: "M12 8a4 4 0 1 0 0 8a4 4 0 0 0 0-8zM12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4",
  moon: "M20.5 13.5A8.5 8.5 0 1 1 10.5 3.5a6.5 6.5 0 0 0 10 10z",
  clock: "M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18zM12 7.5V12l3 2",
  alert: "M12 9.5v4M12 17h.01M10.3 4.6L2.9 17.5a2 2 0 0 0 1.7 3h14.8a2 2 0 0 0 1.7-3L13.7 4.6a2 2 0 0 0-3.4 0z",
  message: "M20.5 12a8 8 0 0 1-8 8H4l2.3-2.9A8 8 0 1 1 20.5 12z",
  pullRequest: "M6 8.5v7M6 8.5a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM6 19.5a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM18 19.5a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM18 15.5V10a3.5 3.5 0 0 0-3.5-3.5H12M14 4l-2.5 2.5L14 9",
  sparkle: "M12 3.5l1.9 5.6l5.6 1.9l-5.6 1.9L12 18.5l-1.9-5.6L4.5 11l5.6-1.9z",
  pencil: "M4 20h4L19.5 8.5l-4-4L4 16z",
  book: "M5 4.5h11a3 3 0 0 1 3 3V20H8a3 3 0 0 1-3-3zM5 17a3 3 0 0 1 3-3h11",
  eye: "M2.5 12S6 5.5 12 5.5S21.5 12 21.5 12S18 18.5 12 18.5S2.5 12 2.5 12zM12 9.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 0 0 0-5z",
  keyboard: "M4 6h16a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2zM6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7 14h10",
  stack: "M12 3.5l9 4.5l-9 4.5L3 8zM3 12.5l9 4.5l9-4.5M3 16.5l9 4.5l9-4.5",
  refresh: "M20 5v5h-5M4 19v-5h5M19.4 10A8 8 0 0 0 5.3 7.6M4.6 14a8 8 0 0 0 14.1 2.4",
  popOut: "M14 4h6v6M20 4l-8 8M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}
