import { resolve } from "node:path";

import react from "@vitejs/plugin-react";
import { defineConfig } from "electron-vite";

// main 과 preload 는 CommonJS 로 빌드한다. sandbox 가 켜진 preload 는 ESM 을 쓸 수 없다.
export default defineConfig({
  main: {
    build: {
      rollupOptions: {
        input: { index: resolve(import.meta.dirname, "src/main/index.ts") },
      },
    },
  },
  preload: {
    build: {
      rollupOptions: {
        input: { index: resolve(import.meta.dirname, "src/preload/index.ts") },
      },
    },
  },
  renderer: {
    root: resolve(import.meta.dirname, "src/renderer"),
    build: {
      rollupOptions: {
        input: {
          index: resolve(import.meta.dirname, "src/renderer/index.html"),
          // UX 블루프린트 화면이다 (`npm run blueprint`). mock 데이터만 쓴다.
          blueprint: resolve(import.meta.dirname, "src/renderer/blueprint.html"),
        },
      },
    },
    plugins: [react()],
  },
});
