// 패널이 닫혀 있어도 글의 진행과 읽지 않은 결과를 안다 (plan 31 B4). 커널의 변경 알림마다 목록을 다시 읽는다.
import { useEffect, useRef, useState } from "react";

import type { RepositoryExtensions } from "./repositoryExtensions";
import { type BlogActivity, blogActivity, finishedArticles, type Summary } from "./techBlogStages";

const BLOG = "akra.tech-blog";
const IDLE: BlogActivity = { running: 0, unread: 0, stopped: 0, next: null };

export function useBlogActivity(extensions: RepositoryExtensions, onFinished: (article: Summary) => void): BlogActivity {
  const [activity, setActivity] = useState(IDLE);
  const previous = useRef<{ kernel: string | null; list: Summary[] }>({ kernel: null, list: [] });
  const notify = useRef(onFinished);
  notify.current = onFinished;
  const enabled = extensions.enabledIds.includes(BLOG);
  useEffect(() => {
    if (!enabled) { setActivity(IDLE); previous.current = { kernel: null, list: [] }; return; }
    let active = true;
    extensions.invoke<Summary[]>(BLOG, "list").then(list => {
      if (!active || !Array.isArray(list)) return;
      // 저장소를 바꾼 뒤 처음 읽은 목록으로는 알리지 않는다. 앞의 목록이 다른 저장소의 것이다.
      if (previous.current.kernel === extensions.kernel) finishedArticles(previous.current.list, list).forEach(a => notify.current(a));
      previous.current = { kernel: extensions.kernel, list };
      setActivity(blogActivity(list));
    }, () => {});
    return () => { active = false; };
  }, [enabled, extensions.invoke, extensions.kernel, extensions.epoch]);
  return activity;
}
