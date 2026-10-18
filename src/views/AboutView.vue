<script setup lang="ts">
/**
 * 关于页：品牌、功能说明、技术信息与开源声明。
 * 版本号复用 check_update 的 current 字段，取不到时回退 package.json 的版本（构建期注入）。
 */
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity, BookOpen, Box, Cpu, FolderOpen, Info, Layers, ScrollText, ShieldCheck,
  Terminal, type LucideIcon,
} from "lucide-vue-next";
import AppMark from "../components/AppMark.vue";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import { getDirs, type Dirs } from "../lib/tauri";
import { toast } from "../lib/toast";

/** check_update 实际返回 snake_case：current / latest / has_update / url / notes */
interface UpdateProbe {
  current: string;
  latest: string;
  has_update?: boolean;
  url?: string;
  notes?: string;
}

interface EnvProbe {
  is_admin: boolean;
  has_webview2: boolean;
}

const version = ref(`v${__APP_VERSION__}`);
const dirs = ref<Dirs | null>(null);
const env = ref<EnvProbe | null>(null);

/** 功能说明：四块能力，两列布局 */
const FEATURES: { title: string; icon: LucideIcon; items: string[] }[] = [
  {
    title: "软件管理",
    icon: Box,
    items: [
      "软件库分类浏览、搜索与批量勾选安装",
      "在线清单源 / 本地目录源统一管理",
      "本机已安装软件列表、卸载与残留清理",
      "任务队列、装机模板与黑白名单保护",
    ],
  },
  {
    title: "硬件检测",
    icon: Cpu,
    items: [
      "CPU / 内存 / 显卡 / 磁盘完整规格",
      "实时占用、进程管理与网络速率",
      "磁盘空间分析、大文件与重复文件扫描",
      "SMART 健康、温度与压力测试",
    ],
  },
  {
    title: "系统工具",
    icon: Terminal,
    items: [
      "启动项、服务、Hosts 与环境变量",
      "快捷命令（SFC / DISM / 网络诊断）",
      "磁盘清理、失效快捷方式与系统信息导出",
      "许可证状态与驱动清单查询",
    ],
  },
  {
    title: "日志与帮助",
    icon: ScrollText,
    items: [
      "运行日志与结构化日志检索",
      "错误码速查与推荐排查步骤",
      "帮助文档与配置备份恢复",
      "定时备份与日志自动清理",
    ],
  },
];

const OPEN_SOURCE = [
  { name: "Tauri", desc: "桌面应用容器" },
  { name: "Vue", desc: "前端框架" },
  { name: "TailwindCSS", desc: "样式工具链" },
  { name: "Radix Vue", desc: "无样式交互组件" },
  { name: "lucide", desc: "图标集" },
  { name: "sysinfo", desc: "系统信息采集" },
  { name: "reqwest", desc: "HTTP 客户端" },
  { name: "serde", desc: "序列化" },
];

async function load() {
  const [upd, d, e] = await Promise.all([
    invoke<UpdateProbe>("check_update").catch(() => null),
    getDirs(),
    invoke<EnvProbe>("env_status").catch(() => null),
  ]);
  if (upd?.current) version.value = `v${upd.current.replace(/^v/i, "")}`;
  dirs.value = d;
  env.value = e;
}

async function openDir(key: "config" | "log") {
  const p = dirs.value?.[key];
  if (!p) {
    toast("目录信息尚未读取完成", "info");
    return;
  }
  try {
    await invoke("open_path", { path: p });
  } catch (err) {
    toast(String(err), "error");
  }
}

onMounted(load);
</script>

<template>
  <div>
    <PageHeader title="关于" desc="版本信息、功能范围与技术栈说明" />

    <!-- 顶部品牌卡 -->
    <Card class="mb-4 p-6">
      <div class="flex flex-wrap items-center gap-4">
        <AppMark :size="64" class="rounded-2xl shadow-lg" />
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <h2 class="text-xl font-bold text-[var(--text-strong)]">极维工具箱</h2>
            <Badge tone="primary">{{ version }}</Badge>
          </div>
          <p class="mt-1 text-sm leading-relaxed text-[var(--muted)]">
            面向装机与运维场景的 Windows 桌面工具箱：软件批量安装、硬件检测、系统维护与日志排查一站式完成。
          </p>
        </div>
      </div>
    </Card>

    <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <!-- 功能说明 -->
      <Card class="p-5 lg:col-span-2">
        <div class="mb-3 flex flex-wrap items-center gap-2">
          <Layers class="h-4 w-4 shrink-0 text-[var(--primary)]" />
          <h4 class="font-semibold text-[var(--text-strong)]">功能说明</h4>
        </div>
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <div
            v-for="f in FEATURES"
            :key="f.title"
            class="min-w-0 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3.5"
          >
            <p class="mb-2 flex items-center gap-1.5 text-sm font-semibold text-[var(--text-strong)]">
              <component :is="f.icon" class="h-4 w-4 shrink-0 text-[var(--primary)]" />
              <span class="min-w-0 truncate">{{ f.title }}</span>
            </p>
            <ul class="space-y-1.5">
              <li
                v-for="(item, i) in f.items"
                :key="i"
                class="flex min-w-0 gap-2 text-xs leading-relaxed text-[var(--muted)]"
              >
                <span class="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-[var(--primary)]" />
                <span class="min-w-0 break-words">{{ item }}</span>
              </li>
            </ul>
          </div>
        </div>
      </Card>

      <!-- 技术信息 -->
      <Card class="p-5">
        <div class="mb-3 flex flex-wrap items-center gap-2">
          <Info class="h-4 w-4 shrink-0 text-[var(--primary)]" />
          <h4 class="font-semibold text-[var(--text-strong)]">技术信息</h4>
        </div>
        <div class="space-y-3 text-xs">
          <div class="flex flex-wrap items-center gap-1.5">
            <span class="shrink-0 text-[var(--muted)]">技术栈</span>
            <Badge tone="gray">Tauri 2</Badge>
            <Badge tone="gray">Vue 3</Badge>
            <Badge tone="gray">Rust</Badge>
          </div>

          <div class="flex flex-wrap items-center gap-1.5">
            <span class="shrink-0 text-[var(--muted)]">系统架构</span>
            <template v-if="env">
              <Badge :tone="env.is_admin ? 'success' : 'gray'">
                <ShieldCheck class="h-3 w-3" /> {{ env.is_admin ? "管理员权限" : "普通权限" }}
              </Badge>
              <Badge :tone="env.has_webview2 ? 'success' : 'danger'">
                <Activity class="h-3 w-3" /> WebView2 {{ env.has_webview2 ? "已安装" : "未安装" }}
              </Badge>
            </template>
            <span v-else class="text-[var(--muted)]">读取中…</span>
          </div>

          <div class="min-w-0">
            <p class="mb-1 text-[var(--muted)]">配置目录</p>
            <p class="break-all rounded-lg bg-[var(--card-soft)] px-2 py-1.5 font-mono text-[11px] text-[var(--text)]">
              {{ dirs?.config || "读取中…" }}
            </p>
          </div>

          <div class="flex flex-wrap gap-2 pt-1">
            <Button variant="gray" size="sm" :disabled="!dirs" @click="openDir('config')">
              <FolderOpen class="h-3.5 w-3.5" /> 打开配置目录
            </Button>
            <Button variant="gray" size="sm" :disabled="!dirs" @click="openDir('log')">
              <FolderOpen class="h-3.5 w-3.5" /> 打开日志目录
            </Button>
          </div>
        </div>
      </Card>

      <!-- 开源声明 -->
      <Card class="p-5">
        <div class="mb-3 flex flex-wrap items-center gap-2">
          <BookOpen class="h-4 w-4 shrink-0 text-[var(--primary)]" />
          <h4 class="font-semibold text-[var(--text-strong)]">开源声明</h4>
        </div>
        <p class="mb-3 text-xs leading-relaxed text-[var(--muted)]">
          本工具的实现依赖以下开源项目，感谢它们的作者与社区：
        </p>
        <div class="flex flex-wrap gap-1.5">
          <Badge v-for="o in OPEN_SOURCE" :key="o.name" tone="gray" :title="o.desc">
            {{ o.name }}
          </Badge>
        </div>
        <p class="mt-3 text-[11px] leading-relaxed text-[var(--muted)]">
          各依赖按其原始许可证分发，具体条款见对应项目的 LICENSE 文件。
        </p>
      </Card>
    </div>
  </div>
</template>
