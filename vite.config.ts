import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { readFileSync } from "node:fs";

// 版本号唯一来源：package.json。构建期注入，界面与安装包不再各写一份。
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf-8")) as {
  version: string;
};

// https://vitejs.dev/config/
export default defineConfig({
  // Tauri 需要以相对路径加载资源，勿改
  base: "./",
  plugins: [vue()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        // 把第三方依赖单独切出来：体积稳定、可长期缓存，主包改代码不必重下
        manualChunks: {
          vue: ["vue", "pinia"],
          radix: ["radix-vue"],
          icons: ["lucide-vue-next"],
        },
      },
    },
  },
});
