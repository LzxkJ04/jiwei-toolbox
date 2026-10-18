import { defineStore } from "pinia";
import {
  Activity, Archive, BadgeCheck, Database, FileSpreadsheet, FileText, Gauge, HardDrive,
  AlarmClock, BookOpen, LayoutDashboard, LayoutGrid, LayoutTemplate, Link2Off, ListChecks,
  Info, ServerCog, Settings, ShieldBan, ShieldAlert, Star, Terminal, Wrench, Zap,
  type LucideIcon,
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
  | "filter"
  | "sources"
  | "tasks"
  | "templates"
  | "backup"
  | "autobackup"
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
  | "structuredlog"
  | "errorcodes"
  | "help"
  | "about"
  | "settings";

export interface NavGroup {
  /** 分组标题；空串表示不显示标题（顶部快捷入口） */
  title: string;
  items: NavItem[];
}

/** 侧栏分组：25 个页面平铺会太长，按职责分组 */
export const NAV_GROUPS: NavGroup[] = [
  {
    title: "",
    items: [
      { id: "workbench", label: "快速工作台", icon: LayoutDashboard },
      { id: "library", label: "软件库", icon: LayoutGrid },
      { id: "favorites", label: "我的收藏", icon: Star },
    ],
  },
  {
    title: "软件管理",
    items: [
      { id: "local", label: "本机软件", icon: HardDrive },
      { id: "sources", label: "软件源管理", icon: ServerCog },
      { id: "filter", label: "黑白名单", icon: ShieldBan },
      { id: "tasks", label: "任务队列", icon: ListChecks },
      { id: "templates", label: "任务模板", icon: LayoutTemplate },
      { id: "backup", label: "软件备份", icon: Archive },
      { id: "autobackup", label: "定时备份", icon: AlarmClock },
    ],
  },
  {
    title: "检测与工具",
    items: [
      { id: "hardware", label: "硬件检测", icon: Gauge },
      { id: "monitor", label: "系统监控", icon: Activity },
      { id: "diskspace", label: "磁盘空间", icon: Database },
      { id: "systools", label: "系统工具", icon: Wrench },
      { id: "quicktools", label: "快捷工具", icon: Zap },
      { id: "shellcmd", label: "快捷命令", icon: Terminal },
      { id: "shortcuts", label: "失效快捷方式", icon: Link2Off },
      { id: "report", label: "系统信息导出", icon: FileSpreadsheet },
    ],
  },
  {
    title: "日志与帮助",
    items: [
      { id: "license", label: "许可证状态", icon: BadgeCheck },
      { id: "logs", label: "日志中心", icon: FileText },
      { id: "structuredlog", label: "结构化日志", icon: ListChecks },
      { id: "errorcodes", label: "错误码", icon: ShieldAlert },
      { id: "help", label: "帮助文档", icon: BookOpen },
      { id: "about", label: "关于", icon: Info },
      { id: "settings", label: "设置", icon: Settings },
    ],
  },
];

/** 扁平列表，供命令面板与路由校验使用 */
export const NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items);

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
