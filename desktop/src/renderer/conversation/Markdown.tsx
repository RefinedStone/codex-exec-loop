// 에이전트 메시지의 마크다운 표시이다 (P3 §6.3). react-markdown 은 원시 HTML 을 그리지 않는다.
// 링크는 새 창으로 열어 main 의 setWindowOpenHandler 가 기본 브라우저로 넘기게 한다 (앱 화면은 이동하지 않는다).
import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

const COMPONENTS: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noreferrer">
      {children}
    </a>
  ),
};

const REMARK_PLUGINS = [remarkGfm];

/**
 * 같은 텍스트면 다시 그리지 않는다. 완료된 메시지는 한 번만 파싱되고, 스트리밍 중인 메시지만 다시 파싱된다.
 * shortcut: 스트리밍 중에는 메시지 전체를 다시 파싱한다. 아주 긴 응답에서 느려지면 완료된 블록을 나눠 캐시한다.
 */
export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
