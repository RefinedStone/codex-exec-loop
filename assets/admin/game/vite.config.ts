import { defineConfig } from "vite";

export default defineConfig({
  build: {
    emptyOutDir: true,
    lib: {
      entry: "src/akra-diorama.ts",
      formats: ["iife"],
      fileName: () => "akra-diorama.js",
      name: "AkraAdminDioramaBundle",
    },
    minify: false,
    outDir: "dist",
    sourcemap: false,
    target: "es2020",
  },
});
