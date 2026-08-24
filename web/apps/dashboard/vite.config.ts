import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import { fileURLToPath, URL } from "node:url";

// 构建输出到 telemux crate 的 web/dist，供 include_dir! 编译期嵌入。
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  build: {
    outDir: "../../../crates/telemux/web/dist",
    emptyOutDir: true,
  },
  server: {
    port: 5181,
    proxy: {
      "/api": "http://127.0.0.1:8080",
    },
  },
});
