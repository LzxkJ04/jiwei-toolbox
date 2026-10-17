import { defineStore } from "pinia";
import { listInstalled, listSoftware, type InstalledApp, type SoftwareItem } from "../lib/tauri";

/** 软件库数据：远程清单 + 本机安装状态匹配 */
export const useSoftwareStore = defineStore("software", {
  state: () => ({
    items: null as SoftwareItem[] | null,
    installed: [] as InstalledApp[],
    loading: false,
  }),
  getters: {
    /** 授权标签：清单 license 字段优先，缺失时按 notes/url 关键词推断并优雅降级 */
    licenseOf: () => (s: SoftwareItem): "free" | "open" | "commercial" => {
      if (s.license === "free" || s.license === "open" || s.license === "commercial") {
        return s.license;
      }
      const text = `${s.notes ?? ""} ${s.url ?? ""}`.toLowerCase();
      if (/开源|open.?source|gpl|mit|apache/.test(text)) return "open";
      if (/商业|收费|付费|license|激活|purchase/.test(text)) return "commercial";
      return "free";
    },
    /** 静默安装是否支持：silentArgs 缺失视为不支持 */
    silentSupported: () => (s: SoftwareItem): boolean => !!(s.silentArgs && s.silentArgs.trim()),
    /** 与本机安装列表匹配（名称包含，大小写不敏感） */
    installedOf(state) {
      const list = state.installed;
      return (s: SoftwareItem): InstalledApp | null => {
        if (!s.name) return null;
        const n = s.name.toLowerCase();
        return (
          list.find(
            (a) =>
              a.name?.toLowerCase().includes(n) ||
              (n.length >= 4 && a.name?.toLowerCase().includes(n.split(" ")[0]))
          ) ?? null
        );
      };
    },
    /** 状态：installed / none（可升级状态待后端提供版本比对后启用） */
    statusOf() {
      return (s: SoftwareItem): "installed" | "upgradable" | "none" => {
        return this.installedOf(s) ? "installed" : "none";
      };
    },
    categories(state): string[] {
      const set = new Set<string>();
      (state.items ?? []).forEach((i) => i.category && set.add(i.category));
      return Array.from(set);
    },
  },
  actions: {
    async load() {
      if (this.loading) return;
      this.loading = true;
      const [items, installed] = await Promise.all([listSoftware(), listInstalled()]);
      if (items !== null) this.items = items;
      if (installed !== null) this.installed = installed;
      this.loading = false;
    },
  },
});
