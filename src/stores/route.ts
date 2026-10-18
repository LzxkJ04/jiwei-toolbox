import { defineStore } from "pinia";
import {
  Activity, Archive, BadgeCheck, Database, FileSpreadsheet, FileText, Gauge, HardDrive,
  LayoutDashboard, LayoutGrid, LayoutTemplate, Link2Off, ListChecks, Settings, Star,
  Terminal, Wrench, Zap, type LucideIcon,
} from "lucide-vue-next";

export interface NavItem {
  id: PageId;
  label: string;
  icon: LucideIcon;
}

export type PageId =
  | "workbench"
  | "library"
  | "favorites"
  | "local"
  | "tasks"
  | "templates"
  | "backup"
  | "hardware"
  | "monitor"
  | "diskspace"
  | "quicktools"
  | "shellcmd"
  | "shortcuts"
  | "report"
  | "systools"
  | "license"
  | "logs"
  | "settings";

export const NAV_ITEMS: NavItem[] = [
  { id: "workbench", label: "快速工作台", icon: LayoutDashboard },
  { id: "library", label: "软件库", icon: LayoutGrid },
  { id: "favorites", label: "我的收藏", icon: Star },
  { id: "local", label: "本机软件", icon: HardDrive },
  { id: "tasks", label: "任务队列", icon: ListChecks },
  { id: "templates", label: "任务模板", icon: LayoutTemplate },
  { id: "backup", label: "软件备份", icon: Archive },
  { id: "hardware", label: "硬件检测", icon: Gauge },
  { id: "monitor", label: "系统监控", icon: Activity },
  { id: "diskspace", label: "磁盘空间", icon: Database },
  { id: "quicktools", label: "快捷工具", icon: Zap },
  { id: "shellcmd", label: "快捷命令", icon: Terminal },
  { id: "shortcuts", label: "失效快捷方式", icon: Link2Off },
  { id: "report", label: "系统信息导出", icon: FileSpreadsheet },
  { id: "systools", label: "系统工具", icon: Wrench },
  { id: "license", label: "许可证状态", icon: BadgeCheck },
  { id: "logs", label: "日志中心", icon: FileText },
  { id: "settings", label: "设置", icon: Settings },
];

export const useRouteStore = defineStore("route", {
  state: () => ({
    active: "workbench" as PageId,
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
