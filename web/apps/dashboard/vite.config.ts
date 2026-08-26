import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import { fileURLToPath, URL } from "node:url";

// 构建输出到 telemux crate 的 web/dist，供 include_dir! 编译期嵌入。
// `@/` 别名指向 packages/ui/src：共享包内 shadcn 组件源码用 `@/lib/utils`、
// `@/components/ui/*` 相互导入，需在打包时正确解析。
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("../../packages/ui/src", import.meta.url)),
    },
  },
  build: {
    outDir: "../../../crates/telemux/web/dist",
    emptyOutDir: true,
  },
  server: {
    port: 5181,
    proxy: {
      "/api": {
        target: "http://127.0.0.1:8080",
        ws: true, // /api/ws WebSocket 升级需要代理转发
      },
    },
  },
});
