import { defineStore } from "pinia";
import {
  Archive, BadgeCheck, FileText, Gauge, HardDrive, LayoutGrid,
  ListChecks, Settings, Star, Wrench, type LucideIcon,
} from "lucide-vue-next";

export interface NavItem {
  id: PageId;
  label: string;
  icon: LucideIcon;
}

export type PageId =
  | "library"
  | "favorites"
  | "local"
  | "tasks"
  | "backup"
  | "hardware"
  | "systools"
  | "license"
  | "logs"
  | "settings";

export const NAV_ITEMS: NavItem[] = [
  { id: "library", label: "软件库", icon: LayoutGrid },
  { id: "favorites", label: "我的收藏", icon: Star },
  { id: "local", label: "本机软件", icon: HardDrive },
  { id: "tasks", label: "任务队列", icon: ListChecks },
  { id: "backup", label: "软件备份", icon: Archive },
  { id: "hardware", label: "硬件检测", icon: Gauge },
  { id: "systools", label: "系统工具", icon: Wrench },
  { id: "license", label: "许可证状态", icon: BadgeCheck },
  { id: "logs", label: "日志中心", icon: FileText },
  { id: "settings", label: "设置", icon: Settings },
];

export const useRouteStore = defineStore("route", {
  state: () => ({
    active: "library" as PageId,
    collapsed: false,
  }),
  actions: {
    go(id: PageId) {
      if (NAV_ITEMS.some((n) => n.id === id)) this.active = id;
      else console.warn(`[navigate] 未知页面: ${id}`);
    },
    toggleCollapsed() {
      this.collapsed = !this.collapsed;
    },
  },
});
