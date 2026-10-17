import { defineStore } from "pinia";
import { getSetting, setSetting } from "../lib/tauri";

export type ThemeMode = "dark" | "light" | "system";
export type Accent = "blue" | "cyan" | "purple" | "orange";

export const ACCENTS: { id: Accent; label: string; color: string }[] = [
  { id: "blue", label: "极维蓝", color: "#3b82f6" },
  { id: "cyan", label: "深海青", color: "#06b6d4" },
  { id: "purple", label: "幻影紫", color: "#8b5cf6" },
  { id: "orange", label: "活力橙", color: "#f97316" },
];

const LS_MODE = "jw-theme-mode";
const LS_ACCENT = "jw-accent";

function applyDom(mode: ThemeMode, accent: Accent) {
  const resolved = mode === "system" ? resolveSystem() : mode;
  document.documentElement.setAttribute("data-theme", resolved);
  document.documentElement.setAttribute("data-accent", accent);
}

function resolveSystem(): "dark" | "light" {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch {
    return "dark";
  }
}

export const useThemeStore = defineStore("theme", {
  state: () => ({
    mode: "dark" as ThemeMode,
    accent: "blue" as Accent,
    _initialized: false,
  }),
  getters: {
    resolved: (s): "dark" | "light" => (s.mode === "system" ? resolveSystem() : s.mode),
  },
  actions: {
    /** 初始化：后端设置 > 本地缓存 > 默认深色 */
    async init() {
      if (this._initialized) return;
      this._initialized = true;

      let mode: ThemeMode | null = null;
      let accent: Accent | null = null;
      // 优先后端持久化
      const bm = await getSetting("theme_mode");
      if (bm === "dark" || bm === "light" || bm === "system") mode = bm;
      const ba = await getSetting("accent");
      if (ba === "blue" || ba === "cyan" || ba === "purple" || ba === "orange") accent = ba;
      // 本地缓存兜底
      try {
        const lm = localStorage.getItem(LS_MODE);
        if (!mode && (lm === "dark" || lm === "light" || lm === "system")) mode = lm;
        const la = localStorage.getItem(LS_ACCENT);
        if (!accent && (la === "blue" || la === "cyan" || la === "purple" || la === "orange")) {
          accent = la;
        }
      } catch {
        /* ignore */
      }
      this.mode = mode ?? "dark";
      this.accent = accent ?? "blue";
      applyDom(this.mode, this.accent);

      // 跟随系统：监听系统主题变化
      try {
        window
          .matchMedia("(prefers-color-scheme: dark)")
          .addEventListener("change", () => {
            if (this.mode === "system") applyDom(this.mode, this.accent);
          });
      } catch {
        /* ignore */
      }
    },
    setMode(m: ThemeMode) {
      this.mode = m;
      applyDom(m, this.accent);
      try {
        localStorage.setItem(LS_MODE, m);
      } catch {
        /* ignore */
      }
      void setSetting("theme_mode", m);
    },
    setAccent(a: Accent) {
      this.accent = a;
      applyDom(this.mode, a);
      try {
        localStorage.setItem(LS_ACCENT, a);
      } catch {
        /* ignore */
      }
      void setSetting("accent", a);
    },
  },
});
