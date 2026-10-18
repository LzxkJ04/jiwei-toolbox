<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  ArrowDownWideNarrow, CheckCircle2, Copy, Database, Download, ExternalLink, FolderOpen,
  Loader2, ListChecks, PlayCircle, RefreshCw, Search, Trash2, XCircle, Zap,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Dialog from "../components/ui/Dialog.vue";
import Checkbox from "../components/ui/Checkbox.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import type { InstalledApp, ResidualItem, UwpApp } from "../lib/tauri";
import {
  listInstalled, listUwp, openPath, removeResidual, removeUwp, runUninstaller, scanResidual,
  processRunning, killProcess,
} from "../lib/tauri";
import { copyText, formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

/* ============ 数据加载 ============ */

const apps = ref<InstalledApp[] | null>(null);
const uwpApps = ref<UwpApp[] | null>(null);
const refreshing = ref(false);

async function refresh() {
  refreshing.value = true;
  const [a, u] = await Promise.all([listInstalled(), listUwp()]);
  apps.value = a ?? [];
  uwpApps.value = u ?? [];
  refreshing.value = false;
}
onMounted(refresh);

/* ============ 顶部筛选 ============ */

const search = ref("");
const filter = ref("all");
const filterOptions = [
  { value: "all", label: "全部" },
  { value: "desktop", label: "桌面软件" },
  { value: "uwp", label: "UWP 应用" },
  { value: "silent", label: "可静默卸载" },
];

/** 可静默卸载估算：卸载器路径含 msiexec / quiet / silent / unins 关键字 */
function isSilentCapable(app: InstalledApp): boolean {
  return /msiexec|quiet|silent|unins/i.test(app.uninstaller);
}

/** 从卸载器路径估算安装目录（后端 list_installed 无 install_location 字段） */
function estimateInstallDir(app: InstalledApp): string {
  const u = app.uninstaller.trim();
  if (!u || /^msiexec/i.test(u)) return "";
  let p: string;
  if (u.startsWith('"')) {
    const end = u.indexOf('"', 1);
    if (end <= 1) return "";
    p = u.slice(1, end);
  } else {
    const sp = u.indexOf(" /");
    p = sp > 0 ? u.slice(0, sp) : u;
  }
  const dir = p.replace(/[\\/][^\\/]+$/, "");
  return /^[a-z]:[\\/]/i.test(dir) ? dir : "";
}

const showDesktop = computed(() => filter.value !== "uwp");
const showUwp = computed(() => filter.value === "all" || filter.value === "uwp");

/** 排序方式：名称 / 体积 / 发行商 */
const sortBy = ref("name");
const sortOptions = [
  { value: "name", label: "按名称" },
  { value: "size", label: "按体积" },
  { value: "publisher", label: "按发行商" },
];

function sortList<T>(list: T[], key: (item: T) => string | number): T[] {
  const arr = list.slice();
  const dir = sortBy.value === "name" ? 1 : -1;
  arr.sort((a, b) => {
    const ka = key(a);
    const kb = key(b);
    if (typeof ka === "number" && typeof kb === "number") return (ka - kb) * dir;
    return String(ka).localeCompare(String(kb), "zh-CN") * dir;
  });
  return arr;
}

const visibleApps = computed(() => {
  let list = apps.value ?? [];
  if (filter.value === "silent") list = list.filter(isSilentCapable);
  const q = search.value.trim().toLowerCase();
  if (q) {
    list = list.filter((a) =>
      `${a.name} ${a.publisher}`.toLowerCase().includes(q)
    );
  }
  if (sortBy.value === "size") return sortList(list, (a) => a.size_estimate || 0);
  if (sortBy.value === "publisher") return sortList(list, (a) => a.publisher || "");
  return sortList(list, (a) => a.name || "");
});

const visibleUwp = computed(() => {
  if (!showUwp.value) return [];
  let list = uwpApps.value ?? [];
  const q = search.value.trim().toLowerCase();
  if (q) list = list.filter((a) => a.name.toLowerCase().includes(q));
  return sortList(list, (a) => a.name);
});

const loading = computed(() => apps.value === null && refreshing.value);

/** 已安装软件占用汇总 */
const totalSize = computed(() =>
  (apps.value ?? []).reduce((s, a) => s + (a.size_estimate || 0), 0)
);
const silentCount = computed(() => (apps.value ?? []).filter(isSilentCapable).length);

/* ============ 导出 / 复制清单 ============ */

function csvCell(v: string | number | undefined) {
  const s = String(v ?? "");
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** 导出为 CSV（Excel 可直接打开，带 BOM 免得中文乱码） */
function exportCsv() {
  const rows = visibleApps.value;
  if (rows.length === 0) {
    toast("当前没有可导出的软件", "info");
    return;
  }
  const header = ["名称", "版本", "发行商", "卸载命令", "注册表键", "估算体积(MB)", "安装目录"];
  const lines = [header.join(",")];
  for (const a of rows) {
    lines.push(
      [
        csvCell(a.name),
        csvCell(a.version),
        csvCell(a.publisher),
        csvCell(a.uninstaller),
        csvCell(a.key),
        csvCell(a.size_estimate),
        csvCell((a.install_location || "").trim() || estimateInstallDir(a)),
      ].join(",")
    );
  }
  const blob = new Blob(["\uFEFF" + lines.join("\r\n")], { type: "text/csv;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `本机软件-${new Date().toISOString().slice(0, 10)}.csv`;
  a.click();
  URL.revokeObjectURL(a.href);
  toast(`已导出 ${rows.length} 条软件记录`, "success");
}

/** 复制纯文本清单，便于粘贴到聊天或文档 */
async function copyList() {
  const rows = visibleApps.value;
  if (rows.length === 0) {
    toast("当前没有可复制的软件", "info");
    return;
  }
  const text = rows
    .map((a) => `- ${a.name}${a.version ? ` ${a.version}` : ""}${a.publisher ? `（${a.publisher}）` : ""}`)
    .join("\n");
  const ok = await copyText(text);
  toast(ok ? `已复制 ${rows.length} 条软件清单` : "复制失败", ok ? "success" : "error");
}

/* ============ 行操作：打开软件目录（降级）/ 注册表信息 ============ */

async function openInstallDir(app: InstalledApp) {
  // 优先用后端记录的安装位置，没有再按卸载器路径估算
  const dir = (app.install_location || "").trim() || estimateInstallDir(app);
  if (!dir) {
    toast("无法确定安装目录（卸载器路径缺失或为 MSI）", "info");
    return;
  }
  try {
    await openPath(dir);
  } catch {
    const ok = await copyText(dir);
    toast(ok ? `目录无法打开，路径已复制：${dir}` : "打开失败", ok ? "warning" : "error");
  }
}

const regApp = ref<InstalledApp | null>(null);

async function copyKey() {
  if (!regApp.value) return;
  const ok = await copyText(regApp.value.key);
  toast(ok ? "注册表路径已复制" : "复制失败", ok ? "success" : "error");
}

/* ============ 卸载前进程检测 ============ */

const procConfirmApp = ref<InstalledApp | null>(null);
const procPids = ref<number[]>([]);
let procConfirmNext: (() => void) | null = null;

/** 卸载前先检测进程：未检测到（或检测失败）直接继续，检测到则弹确认窗 */
async function withProcessCheck(app: InstalledApp, next: () => void) {
  const pids = await processRunning(app.name);
  if (pids === null || pids.length === 0) {
    next();
    return;
  }
  procPids.value = pids;
  procConfirmNext = next;
  procConfirmApp.value = app;
}

/** 【自动关闭】结束进程，全部成功才继续卸载流程 */
async function autoKillAndContinue() {
  const next = procConfirmNext;
  const pids = procPids.value;
  procConfirmApp.value = null;
  procConfirmNext = null;
  if (!next) return;
  const res = await killProcess(pids);
  if (res === null || res.killed.length === 0 || res.failed.length > 0) {
    toast("自动关闭失败，请手动关闭后重试", "warning");
    return;
  }
  toast(`已自动关闭 ${res.killed.length} 个进程`, "success");
  next();
}

/** 【跳过】不关闭进程，直接继续卸载流程 */
function skipProcessKill() {
  const next = procConfirmNext;
  procConfirmApp.value = null;
  procConfirmNext = null;
  next?.();
}

/* ============ 标准卸载（降级：后端无 run_uninstaller 命令） ============ */

const stdConfirmApp = ref<InstalledApp | null>(null);
const stdApp = ref<InstalledApp | null>(null);

/** 步骤①：进程检测 → 确认 */
function startStandardUninstall(app: InstalledApp) {
  void withProcessCheck(app, () => {
    stdConfirmApp.value = app;
  });
}
function onStdConfirmed() {
  stdApp.value = stdConfirmApp.value;
}
/** 步骤②降级：展示卸载器路径 + 复制按钮，引导手动运行；完成后走残留扫描 */
async function copyUninstaller() {
  if (!stdApp.value) return;
  const ok = await copyText(stdApp.value.uninstaller || "");
  toast(ok ? "卸载命令已复制，请在「运行」或终端中手动执行" : "复制失败", ok ? "success" : "error");
}
function scanAfterStd() {
  const app = stdApp.value;
  stdApp.value = null;
  if (app) startForcedUninstall(app);
}

const runningUninstaller = ref(false);

/** 直接启动卸载程序（后端只允许 msiexec /X 或已存在的 exe 路径） */
async function runStdUninstaller() {
  const app = stdApp.value;
  if (!app?.uninstaller || runningUninstaller.value) return;
  runningUninstaller.value = true;
  try {
    const msg = await runUninstaller(app.uninstaller);
    toast(msg, "success");
    stdApp.value = null;
  } catch (err) {
    toast(`启动失败：${String(err)}`, "error");
  } finally {
    runningUninstaller.value = false;
  }
}

/* ============ 强制卸载：进程检测 → 残留扫描 → 预览 → 清理 ============ */

/** 强制卸载按钮入口：先做进程检测，再进入残留扫描流程 */
function onForcedUninstallClick(app: InstalledApp) {
  void withProcessCheck(app, () => startForcedUninstall(app));
}

const residualApp = ref<InstalledApp | null>(null);
const residualOpen = ref(false);
const scanning = ref(false);
const cleaning = ref(false);
const residualItems = ref<ResidualItem[]>([]);
const residualChecked = ref(new Set<string>());

const RESIDUAL_GROUPS: { key: string; label: string }[] = [
  { key: "dir", label: "目录" },
  { key: "file", label: "文件" },
  { key: "reg", label: "注册表" },
];

const residualGroups = computed(() =>
  RESIDUAL_GROUPS.map((g) => ({
    ...g,
    items: residualItems.value.filter((r) => r.kind === g.key),
  })).filter((g) => g.items.length > 0)
);

function toggleResidual(target: string, v: boolean) {
  const next = new Set(residualChecked.value);
  if (v) next.add(target);
  else next.delete(target);
  residualChecked.value = next;
}

async function startForcedUninstall(app: InstalledApp) {
  residualApp.value = app;
  residualOpen.value = true;
  scanning.value = true;
  residualItems.value = [];
  const res = await scanResidual(app.name);
  residualItems.value = res ?? [];
  residualChecked.value = new Set((res ?? []).map((r) => r.target));
  scanning.value = false;
}

async function confirmClean() {
  if (!residualApp.value || residualChecked.value.size === 0) return;
  cleaning.value = true;
  const items = residualItems.value.filter((r) => residualChecked.value.has(r.target));
  const res = await removeResidual(items);
  cleaning.value = false;
  if (res !== null) {
    toast(`已清理 ${items.length} 项残留`, "success");
  }
  residualOpen.value = false;
  void refresh();
}

/* ============ UWP 移除 ============ */

const uwpConfirm = ref<UwpApp | null>(null);
const uwpRemoving = ref(false);

async function onUwpConfirmed() {
  const app = uwpConfirm.value;
  if (!app) return;
  uwpRemoving.value = true;
  const res = await removeUwp(app.package);
  uwpRemoving.value = false;
  if (res !== null) toast(`已移除 ${app.name}`, "success");
  void refresh();
}

/* ============ 批量卸载队列（本地状态，逐个执行强制卸载流程） ============ */

interface UninstallQueueItem {
  name: string;
  status: "pending" | "running" | "done" | "failed";
  msg: string;
}

const checked = ref(new Set<string>());
const uninstallQueue = ref<UninstallQueueItem[]>([]);
const queueRunning = ref(false);
const queuePanelOpen = ref(false);

function setChecked(app: InstalledApp, v: boolean) {
  const next = new Set(checked.value);
  if (v) next.add(app.name);
  else next.delete(app.name);
  checked.value = next;
}

const allChecked = computed(
  () => visibleApps.value.length > 0 && visibleApps.value.every((a) => checked.value.has(a.name))
);

function toggleSelectAll() {
  if (allChecked.value) checked.value = new Set();
  else checked.value = new Set(visibleApps.value.map((a) => a.name));
}

function enqueueChecked() {
  if (checked.value.size === 0) return;
  let added = 0;
  for (const name of checked.value) {
    if (uninstallQueue.value.some((t) => t.name === name && t.status !== "done" && t.status !== "failed")) continue;
    uninstallQueue.value.push({ name, status: "pending", msg: "等待中" });
    added++;
  }
  queuePanelOpen.value = true;
  toast(added > 0 ? `已将 ${added} 项加入卸载队列` : "所选软件均已在卸载队列中", added > 0 ? "success" : "info");
}

async function runQueue() {
  if (queueRunning.value || uninstallQueue.value.length === 0) return;
  queueRunning.value = true;
  for (const task of uninstallQueue.value) {
    if (task.status === "done" || task.status === "failed") continue;
    task.status = "running";
    task.msg = "扫描残留中…";
    const residuals = await scanResidual(task.name);
    if (residuals === null) {
      task.status = "failed";
      task.msg = "残留扫描失败";
      continue;
    }
    if (residuals.length === 0) {
      task.status = "done";
      task.msg = "未发现残留";
      continue;
    }
    task.msg = `清理 ${residuals.length} 项残留…`;
    const res = await removeResidual(residuals);
    if (res === null) {
      task.status = "failed";
      task.msg = "残留清理失败";
    } else {
      task.status = "done";
      task.msg = `已清理 ${residuals.length} 项残留`;
    }
  }
  queueRunning.value = false;
  checked.value = new Set();
  void refresh();
}

function clearQueue() {
  if (queueRunning.value) {
    toast("队列执行中，无法清空", "info");
    return;
  }
  uninstallQueue.value = [];
}
</script>

<template>
  <div>
    <PageHeader title="本机软件" desc="查看与管理本机已安装软件，支持残留扫描与强制卸载">
      <template #extra>
        <Button variant="gray" size="sm" title="导出当前列表为 CSV" :disabled="visibleApps.length === 0" @click="exportCsv">
          <Download class="h-3.5 w-3.5" /> 导出 CSV
        </Button>
        <Button variant="gray" size="sm" title="复制为纯文本清单" :disabled="visibleApps.length === 0" @click="copyList">
          <Copy class="h-3.5 w-3.5" /> 复制清单
        </Button>
      </template>
    </PageHeader>

    <!-- 汇总 -->
    <div class="mb-3 flex flex-wrap items-center gap-x-5 gap-y-1 text-xs text-[var(--muted)]">
      <span>{{ (apps ?? []).length }} 个桌面软件 · {{ (uwpApps ?? []).length }} 个 UWP</span>
      <span>估算占用 {{ formatBytes(totalSize) }}</span>
      <span>可静默卸载 {{ silentCount }} 个</span>
    </div>

    <!-- 顶部栏：搜索 + 筛选 + 刷新 -->
    <div class="mb-4 flex flex-wrap items-center gap-3">
      <div class="relative min-w-[220px] flex-1">
        <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
        <Input v-model="search" placeholder="搜索软件名称或厂商…" class="!pl-9" />
      </div>
      <Select v-model="filter" :options="filterOptions" class="w-36" />
      <div class="flex items-center gap-1.5">
        <ArrowDownWideNarrow class="h-4 w-4 shrink-0 text-[var(--muted)]" />
        <Select v-model="sortBy" :options="sortOptions" class="w-32" />
      </div>
      <Button variant="gray" size="icon" title="刷新列表" :disabled="refreshing" @click="refresh">
        <RefreshCw class="h-4 w-4" :class="refreshing && 'animate-spin'" />
      </Button>
    </div>

    <!-- 加载中 -->
    <div v-if="loading" class="glass-card">
      <Spinner label="正在枚举本机软件…" />
    </div>

    <template v-else>
      <!-- 桌面软件列表 -->
      <div v-if="showDesktop" class="glass-card divide-y divide-[var(--card-border)] !p-0">
        <EmptyState
          v-if="visibleApps.length === 0"
          icon="💻"
          text="没有匹配的桌面软件"
          sub="试试调整搜索词或筛选条件"
        />
        <div
          v-for="app in visibleApps"
          :key="app.key"
          class="flex items-center gap-3 px-4 py-3 transition-colors hover:bg-[var(--card-soft)]"
        >
          <Checkbox :checked="checked.has(app.name)" @update:checked="(v) => setChecked(app, v)" />
          <!-- 首字母方块 -->
          <div
            class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg text-base font-bold"
            :style="{ background: 'var(--primary-soft)', color: 'var(--primary)' }"
          >
            {{ (app.name || "?").slice(0, 1).toUpperCase() }}
          </div>
          <!-- 名称 / 版本 / 厂商 / 路径 -->
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="truncate text-sm font-semibold text-[var(--text-strong)]">{{ app.name }}</span>
              <span v-if="app.version" class="shrink-0 text-xs text-[var(--muted)]">v{{ app.version }}</span>
              <span
                v-if="isSilentCapable(app)"
                class="shrink-0 rounded-full bg-[var(--primary-soft)] px-1.5 py-0.5 text-[10px] text-[var(--primary)]"
                title="卸载器疑似支持静默卸载（按路径关键字估算）"
              >可静默</span>
            </div>
            <div class="mt-0.5 flex items-center gap-2 text-xs text-[var(--muted)]">
              <span class="max-w-[180px] truncate">{{ app.publisher || "未知厂商" }}</span>
              <span v-if="app.size_estimate > 0" class="shrink-0">{{ formatBytes(app.size_estimate) }}</span>
              <span class="min-w-0 truncate" :title="estimateInstallDir(app) || app.uninstaller">
                {{ estimateInstallDir(app) || app.uninstaller || "路径未知" }}
              </span>
            </div>
          </div>
          <!-- 操作按钮组 -->
          <div class="flex shrink-0 items-center gap-1.5">
            <Button variant="ghost" size="sm" title="打开软件所在目录（复制路径）" @click="openInstallDir(app)">
              <FolderOpen class="h-3.5 w-3.5" />
              目录
            </Button>
            <Button variant="ghost" size="sm" title="查看注册表信息" @click="regApp = app">
              <Database class="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="sm" title="标准卸载" :disabled="!app.uninstaller" @click="startStandardUninstall(app)">
              <ExternalLink class="h-3.5 w-3.5" />
            </Button>
            <Button variant="ghost" size="sm" class="!text-[#e5484d]" title="强制卸载（跳过卸载器，直接清理残留）" @click="onForcedUninstallClick(app)">
              <Zap class="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>

      <!-- UWP 应用分区 -->
      <div v-if="showUwp" class="mt-4">
        <h3 class="mb-2 flex items-center gap-2 text-sm font-semibold text-[var(--muted)]">
          UWP 应用
          <span class="text-xs font-normal">{{ visibleUwp.length }} 项</span>
        </h3>
        <div class="glass-card divide-y divide-[var(--card-border)] !p-0">
          <EmptyState
            v-if="visibleUwp.length === 0"
            icon="🪟"
            text="没有匹配的 UWP 应用"
            sub="试试调整搜索词"
          />
          <div
            v-for="app in visibleUwp"
            :key="app.package"
            class="flex items-center gap-3 px-4 py-2.5 transition-colors hover:bg-[var(--card-soft)]"
          >
            <div
              class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-sm font-bold"
              :style="{ background: 'var(--primary-soft)', color: 'var(--primary)' }"
            >
              {{ (app.name || "?").slice(0, 1).toUpperCase() }}
            </div>
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <span class="truncate text-sm font-medium">{{ app.name }}</span>
                <span v-if="app.version" class="shrink-0 text-xs text-[var(--muted)]">v{{ app.version }}</span>
              </div>
              <p class="truncate text-xs text-[var(--muted)]" :title="app.package">{{ app.package }}</p>
            </div>
            <Button variant="ghost" size="sm" class="!text-[#e5484d]" title="移除 UWP 应用" @click="uwpConfirm = app">
              <Trash2 class="h-3.5 w-3.5" />
              移除
            </Button>
          </div>
        </div>
      </div>

      <!-- 底部批量操作栏 -->
      <div
        class="sticky bottom-4 mt-5 flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-[var(--card-border)] bg-[var(--card-solid)]/90 px-5 py-3.5 shadow-glass backdrop-blur-xl"
      >
        <div class="flex items-center gap-4">
          <span class="text-sm text-[var(--muted)]">
            已勾选 <b class="text-[var(--primary)]">{{ checked.size }}</b> 个软件
          </span>
          <span v-if="uninstallQueue.length > 0" class="text-xs text-[var(--muted)]">
            队列中 <b class="text-[var(--text-strong)]">{{ uninstallQueue.length }}</b> 项，逐个执行强制卸载流程
          </span>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="gray" size="sm" @click="toggleSelectAll">
            {{ allChecked ? "取消批量勾选" : "批量勾选" }}
          </Button>
          <Button v-if="uninstallQueue.length > 0" variant="gray" size="sm" @click="queuePanelOpen = !queuePanelOpen">
            {{ queuePanelOpen ? "收起队列" : "查看队列" }}
          </Button>
          <Button variant="danger" size="sm" :disabled="checked.size === 0" @click="enqueueChecked">
            <ListChecks class="h-4 w-4" />
            将选中项加入卸载队列
          </Button>
        </div>
      </div>

      <!-- 卸载队列面板 -->
      <div v-if="queuePanelOpen && uninstallQueue.length > 0" class="mt-4">
        <div class="glass-card divide-y divide-[var(--card-border)] !p-0">
          <div class="flex items-center justify-between px-4 py-2.5">
            <h4 class="text-sm font-semibold">卸载队列</h4>
            <div class="flex gap-2">
              <Button variant="primary" size="sm" :disabled="queueRunning" @click="runQueue">
                <PlayCircle class="h-3.5 w-3.5" />
                {{ queueRunning ? "执行中…" : "开始执行" }}
              </Button>
              <Button variant="gray" size="sm" :disabled="queueRunning" @click="clearQueue">清空</Button>
            </div>
          </div>
          <div v-for="(task, i) in uninstallQueue" :key="task.name" class="flex items-center gap-3 px-4 py-2 text-sm">
            <Loader2 v-if="task.status === 'running'" class="h-4 w-4 shrink-0 animate-spin text-[var(--primary)]" />
            <CheckCircle2 v-else-if="task.status === 'done'" class="h-4 w-4 shrink-0 text-[#30a46c]" />
            <XCircle v-else-if="task.status === 'failed'" class="h-4 w-4 shrink-0 text-[#e5484d]" />
            <span v-else class="h-4 w-4 shrink-0 rounded-full border border-[var(--card-border)]" />
            <span class="min-w-0 flex-1 truncate">{{ i + 1 }}. {{ task.name }}</span>
            <span
              class="shrink-0 text-xs"
              :class="task.status === 'failed' ? 'text-[#e5484d]' : task.status === 'done' ? 'text-[#30a46c]' : 'text-[var(--muted)]'"
            >{{ task.msg }}</span>
          </div>
        </div>
      </div>
    </template>

    <!-- 注册表信息弹窗 -->
    <Dialog :open="!!regApp" title="注册表信息" class="!max-w-lg" @update:open="regApp = null">
      <div v-if="regApp">
        <div class="mb-3 text-sm">
          <p class="mb-1 font-semibold text-[var(--text-strong)]">{{ regApp.name }}</p>
          <p class="text-xs text-[var(--muted)]">v{{ regApp.version || "未知" }} · {{ regApp.publisher || "未知厂商" }}</p>
        </div>
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <p class="mb-1.5 text-xs font-medium text-[var(--muted)]">卸载注册表键（只读）</p>
          <div class="flex items-start gap-2">
            <code class="min-w-0 flex-1 break-all rounded bg-[var(--btn-gray)] px-2 py-1.5 font-mono text-xs">
              {{ regApp.key }}
            </code>
            <Button variant="gray" size="icon" title="复制注册表路径" @click="copyKey">
              <Copy class="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>
    </Dialog>

    <!-- 标准卸载降级弹窗：展示卸载器路径，引导手动运行 -->
    <Dialog :open="!!stdApp" title="标准卸载" class="!max-w-lg" @update:open="stdApp = null">
      <div v-if="stdApp">
        <p class="mb-3 rounded-xl bg-[#f5a6231a] p-3 text-xs leading-relaxed text-[#b57d10]">
          卸载前请确保软件已关闭。点下方按钮直接运行卸载程序，或复制命令自行执行（Win+R 粘贴）。
          卸载完成后可点击「扫描残留」自动清理剩余文件与注册表项。
        </p>
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <p class="mb-1.5 text-xs font-medium text-[var(--muted)]">卸载器（{{ stdApp.name }}）</p>
          <div class="flex items-start gap-2">
            <code class="min-w-0 flex-1 break-all rounded bg-[var(--btn-gray)] px-2 py-1.5 font-mono text-xs">
              {{ stdApp.uninstaller || "（无卸载器信息，建议使用强制卸载）" }}
            </code>
            <Button variant="gray" size="icon" title="复制卸载命令" @click="copyUninstaller">
              <Copy class="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="stdApp = null">关闭</Button>
          <Button
            variant="primary"
            :disabled="!stdApp?.uninstaller || runningUninstaller"
            @click="runStdUninstaller"
          >
            <Loader2 v-if="runningUninstaller" class="h-4 w-4 animate-spin" />
            <PlayCircle v-else class="h-4 w-4" />
            运行卸载程序
          </Button>
          <Button variant="gray" @click="scanAfterStd">
            <Zap class="h-4 w-4" />
            扫描残留
          </Button>
        </div>
      </template>
    </Dialog>

    <!-- 残留预览弹窗 -->
    <Dialog
      :open="residualOpen"
      :title="scanning ? '正在扫描残留…' : `残留预览 · ${residualApp?.name ?? ''}`"
      class="!max-w-2xl"
      @update:open="residualOpen = false"
    >
      <Spinner v-if="scanning" label="正在扫描文件系统与注册表残留…" />
      <EmptyState
        v-else-if="residualItems.length === 0"
        icon="✨"
        text="未扫描到残留"
        sub="该软件在常见安装位置没有发现残留文件或注册表项"
      />
      <div v-else class="space-y-3">
        <p class="text-xs text-[var(--muted)]">已按类型分组，默认全选；请确认后清理。注册表项删除不可恢复。</p>
        <div
          v-for="group in residualGroups"
          :key="group.key"
          class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3"
        >
          <p class="mb-2 text-xs font-semibold">
            {{ group.label }}
            <span class="font-normal text-[var(--muted)]">（{{ group.items.length }} 项）</span>
          </p>
          <div class="space-y-1.5">
            <label
              v-for="r in group.items"
              :key="r.target"
              class="flex cursor-pointer items-start gap-2 text-xs"
            >
              <Checkbox
                class="mt-0.5"
                :checked="residualChecked.has(r.target)"
                @update:checked="(v) => toggleResidual(r.target, v)"
              />
              <span class="min-w-0 break-all font-mono leading-relaxed">{{ r.target }}</span>
            </label>
          </div>
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-3">
          <Button variant="gray" @click="residualOpen = false">跳过清理</Button>
          <Button
            variant="danger"
            :disabled="scanning || residualChecked.size === 0 || cleaning"
            @click="confirmClean"
          >
            <Loader2 v-if="cleaning" class="h-4 w-4 animate-spin" />
            确认清理{{ residualChecked.size > 0 ? `（${residualChecked.size}）` : "" }}
          </Button>
        </div>
      </template>
    </Dialog>

    <!-- UWP 移除确认 -->
    <ConfirmDialog
      :open="!!uwpConfirm"
      title="移除 UWP 应用"
      :message="`确定要移除「${uwpConfirm?.name}」吗？该操作不可恢复。`"
      confirm-text="移除"
      danger
      @confirm="onUwpConfirmed"
      @update:open="uwpConfirm = null"
    />

    <!-- 标准卸载前置确认 -->
    <ConfirmDialog
      :open="!!stdConfirmApp"
      title="标准卸载"
      :message="`即将卸载「${stdConfirmApp?.name}」。卸载前请确保软件已关闭，是否继续？`"
      confirm-text="继续"
      @confirm="onStdConfirmed"
      @update:open="stdConfirmApp = null"
    />

    <!-- 卸载前进程检测：自动关闭 / 跳过 -->
    <ConfirmDialog
      :open="!!procConfirmApp"
      title="检测到软件正在运行"
      :message="`检测到 ${procConfirmApp?.name ?? ''} 正在运行（${procPids.length} 个进程），是否尝试自动关闭进程？`"
      confirm-text="自动关闭"
      cancel-text="跳过"
      danger
      @confirm="autoKillAndContinue"
      @update:open="skipProcessKill"
    />
  </div>
</template>
