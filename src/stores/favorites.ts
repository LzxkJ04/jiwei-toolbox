import { defineStore } from "pinia";
import { getFavorites, getSetting, toggleFavorite } from "../lib/tauri";

const LS_KEY = "jw-favorites";

/** 收藏状态 + 持久化（后端 get_favorites / toggle_favorite，本地缓存兜底） */
export const useFavoritesStore = defineStore("favorites", {
  state: () => ({
    ids: [] as string[],
    loaded: false,
  }),
  getters: {
    count: (s) => s.ids.length,
  },
  actions: {
    async init() {
      if (this.loaded) return;
      this.loaded = true;
      // 1. 后端 settings.json favorites 数组
      let ids = await getFavorites();
      // 2. 兼容旧版：前端曾以 JSON 字符串写入 settings "favorites" / localStorage
      if (ids === null || ids.length === 0) {
        const legacy = (await getSetting("favorites")) ?? null;
        const raw = legacy ?? (() => {
          try {
            return localStorage.getItem(LS_KEY);
          } catch {
            return null;
          }
        })();
        if (raw) {
          try {
            const arr = JSON.parse(raw);
            if (Array.isArray(arr)) ids = arr.filter((x) => typeof x === "string");
          } catch {
            /* ignore */
          }
        }
      }
      if (ids) this.ids = ids;
    },
    isFavorite(id: string): boolean {
      return this.ids.includes(id);
    },
    /** 切换收藏，返回是否为新增收藏 */
    async toggle(id: string): Promise<boolean> {
      const added = !this.isFavorite(id);
      const res = await toggleFavorite(id);
      if (res !== null) {
        this.ids = res;
      } else {
        // 后端失败：本地回退
        const idx = this.ids.indexOf(id);
        if (idx >= 0) this.ids.splice(idx, 1);
        else this.ids.push(id);
      }
      try {
        localStorage.setItem(LS_KEY, JSON.stringify(this.ids));
      } catch {
        /* ignore */
      }
      return added;
    },
  },
});
