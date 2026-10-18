<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  Activity, Cpu, FileSpreadsheet, FolderOpen, HardDrive, LayoutTemplate, ListChecks, Loader2,
  Package, Play, RefreshCw, ShieldCheck, Sparkles, Terminal, Trash2, Wrench,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import {
  getDirs, hwInfo, licenseStatus, loadTemplate, openPath, scanJunk, listTemplates,
  type Dirs, type HwInfo, type JunkItem, type LicenseStatus, type TemplateMeta,
} from "../lib/tauri";
import { useRouteStore } from "../stores/route";
import { useTasksStore } from "../stores/tasks";
import { useSoftwareStore } from "../stores/software";
import { formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

const route = useRouteStore();
const tasks = useTasksStore();
const store = useSoftwareStore();

const hw = ref<HwInfo | null>(null);
const lic = ref<LicenseStatus | null>(null);
const dirs = ref<Dirs | null>(null);
const templates = ref<TemplateMeta[]>([]);
const junk = ref<JunkItem[] | null>(null);
const loading = ref(true);
const scanningJunk = ref(false);
const loadingTpl = ref<string | null>(null);

async function refresh() {
  loading.value = true;
  const [h, l, d, t] = await Promise.all([
    hwInfo(),
    licenseStatus(),
    getDirs(),
    listTemplates(),
  ]);
  hw.value = h;
  lic.value = l;
  dirs.value = d;
  templates.value = t ?? [];
  loading.value = false;
}

/** 本机概况一行摘要 */
const hwSummary = computed(() => {
  const h = hw.value;
  if (!h) return [];
  const gpu = (h.gpus ?? []).find((g) => !g.virtual) ?? h.gpus?.[0];
  return [
    { label: "处理器", value: h.cpu?.name || "—" },
    { label: "内存", value: formatBytes(h.memory_total) },
    { label: "显卡", value: gpu?.name || "—" },
    { label: "系统", value: lic.value?.osEdition || "—" },
  ];
});

const junkTotal = computed(() => (junk.value ?? []).reduce((s, j) => s + (j.size ?? 0), 0));
const recentTasks = computed(() => tasks.tasks.slice(0, 5));

async function doScanJunk() {
  scanningJunk.value = true;
  junk.value = (await scanJunk()) ?? [];
  scanningJunk.value = false;
  toast(junkTotal.value > 0 ? `发现可清理 ${formatBytes(junkTotal.value)}` : "没有发现可清理项", "info");
}

async function openDir(key: "backup" | "cache" | "config" | "log") {
  const p = (dirs.value as unknown as Record<string, string> | null)?.[key];
  if (!p) {
    toast("目录信息尚未读取完成", "info");
    return;
  }
  try {
    await openPath(p);
  } catch (err) {
    toast(String(err), "error");
  }
}

/** 一键加载模板到队列 */
async function useTemplate(name: string) {
  if (loadingTpl.value) return;
  loadingTpl.value = name;
  try {
    const t = await loadTemplate(name);
    await store.load();
    const catalog = store.items ?? [];
    const picked = t.items
      .filter((i) => !i.disabled)
      .map((i) => catalog.find((c) => c.id === i.id))
      .filter((x): x is NonNullable<typeof x> => !!x);
    if (picked.length === 0) {
      toast("模板内软件在当前软件源都找不到", "warning");
      return;
    }
    const n = tasks.addFromSoftware(picked);
    toast(`已加入 ${n} 个软件到任务队列`, n > 0 ? "success" : "info");
    if (t.options.auto_shutdown) await tasks.setFinishAction("shutdown");
    if (t.options.load_action === "start" && n > 0) await tasks.start();
    route.go("tasks");
  } catch (err) {
    toast(`加载失败：${String(err)}`, "error");
  } finally {
    loadingTpl.value = null;
  }
}

/** 快速工具卡片 */
const QUICK_TOOLS = [
  { id: "hardware", label: "硬件概览", desc: "CPU / 内存 / 显卡 / 磁盘", icon: Cpu, page: "hardware" },
  { id: "local", label: "本机软件扫描", desc: "查看已安装软件与卸载", icon: Package, page: "local" },
  { id: "diskspace", label: "磁盘空间分析", desc: "目录占用与重复文件", icon: HardDrive, page: "diskspace" },
  { id: "monitor", label: "系统监控", desc: "实时占用与进程", icon: Activity, page: "monitor" },
  { id: "systools", label: "系统工具", desc: "启动项 / 服务 / Hosts", icon: Wrench, page: "systools" },
  { id: "quicktools", label: "快捷工具", desc: "端口 / 系统入口 / 关机", icon: Sparkles, page: "quicktools" },
  { id: "shellcmd", label: "快捷命令", desc: "SFC / DISM / 网络诊断", icon: Terminal, page: "shellcmd" },
  { id: "report", label: "系统信息导出", desc: "整机报告一键导出", icon: FileSpreadsheet, page: "report" },
] as const;

onMounted(refresh);
</script>

<template>
  <div>
    <PageHeader title="快速工作台" desc="常用功能与装机模板集中在这里，开机先看这一页。">
      <template #extra>
        <Button variant="gray" size="sm" :disabled="loading" @click="refresh">
          <RefreshCw class="h-3.5 w-3.5" :class="loading && 'animate-spin'" /> 刷新
        </Button>
      </template>
    </PageHeader>

    <!-- 欢迎 / 本机概况 -->
    <Card class="mb-4 p-5">
      <Spinner v-if="loading && !hw" label="正在读取本机信息…" />
      <template v-else>
        <div class="mb-3 flex flex-wrap items-center gap-2">
          <h3 class="text-lg font-bold">本机概况</h3>
          <Badge v-if="lic" :tone="lic.licensed ? 'success' : 'warning'">
            <ShieldCheck class="h-3 w-3" /> {{ lic.statusText || "未知" }}
          </Badge>
        </div>
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
          <div v-for="s in hwSummary" :key="s.label" class="rounded-lg bg-[var(--card-soft)] px-3 py-2">
            <p class="text-[10px] text-[var(--muted)]">{{ s.label }}</p>
            <p class="truncate text-sm font-medium" :title="s.value">{{ s.value }}</p>
          </div>
        </div>
      </template>
    </Card>

    <div class="grid grid-cols-1 gap-4 lg:grid-cols-[1fr_340px]">
      <div class="space-y-4">
        <!-- 任务模板快捷卡 -->
        <div>
          <p class="mb-2 flex items-center gap-1.5 text-xs font-semibold text-[var(--text-strong)]">
            <LayoutTemplate class="h-3.5 w-3.5" /> 任务模板
            <button type="button" class="ml-auto text-[10px] font-normal text-[var(--primary)]" @click="route.go('templates')">
              管理模板 →
            </button>
          </p>
          <EmptyState
            v-if="!templates.length"
            icon="📋"
            text="暂无保存的任务模板"
            sub="前往模板管理新建装机模板，以后一键加载"
          />
          <div v-else class="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <div
              v-for="t in templates.slice(0, 4)"
              :key="t.name"
              class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3"
            >
              <p class="truncate text-sm font-semibold">{{ t.name }}</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">{{ t.enabled_count }} 个软件</p>
              <Button
                variant="primary"
                size="sm"
                class="mt-2 w-full"
                :disabled="loadingTpl !== null"
                @click="useTemplate(t.name)"
              >
                <Loader2 v-if="loadingTpl === t.name" class="h-3 w-3 animate-spin" />
                <Play v-else class="h-3 w-3" />
                加载模板
              </Button>
            </div>
          </div>
        </div>

        <!-- 快速工具 -->
        <div>
          <p class="mb-2 flex items-center gap-1.5 text-xs font-semibold text-[var(--text-strong)]">
            <Sparkles class="h-3.5 w-3.5" /> 快速工具
          </p>
          <div class="grid grid-cols-2 gap-2 sm:grid-cols-3">
            <button
              v-for="tool in QUICK_TOOLS"
              :key="tool.id"
              type="button"
              class="flex flex-col items-start gap-1 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3 text-left transition-all hover:-translate-y-0.5 hover:border-[var(--primary)]"
              @click="route.go(tool.page as never)"
            >
              <component :is="tool.icon" class="h-4 w-4 text-[var(--primary)]" />
              <span class="text-sm font-medium">{{ tool.label }}</span>
              <span class="text-[10px] text-[var(--muted)]">{{ tool.desc }}</span>
            </button>
          </div>
        </div>
      </div>

      <!-- 右栏：清理 + 目录 + 最近任务 -->
      <div class="space-y-4">
        <Card class="p-4">
          <div class="mb-2 flex items-center justify-between">
            <p class="flex items-center gap-1.5 text-sm font-semibold">
              <Trash2 class="h-4 w-4 text-[var(--primary)]" /> 垃圾清理
            </p>
            <Button variant="gray" size="sm" :disabled="scanningJunk" @click="doScanJunk">
              <Loader2 v-if="scanningJunk" class="h-3 w-3 animate-spin" />
              <RefreshCw v-else class="h-3 w-3" />
              {{ junk ? "重扫" : "扫描" }}
            </Button>
          </div>
          <p v-if="junk" class="text-xs text-[var(--muted)]">
            发现 {{ junk.length }} 项，可释放
            <b class="text-[var(--primary)]">{{ formatBytes(junkTotal) }}</b>
          </p>
          <p v-else class="text-xs text-[var(--muted)]">点「扫描」检查临时文件与浏览器缓存</p>
          <Button
            v-if="junk && junk.length"
            variant="primary"
            size="sm"
            class="mt-2 w-full"
            @click="route.go('systools')"
          >
            前往清理
          </Button>
        </Card>

        <Card class="p-4">
          <p class="mb-2 text-sm font-semibold">快速打开目录</p>
          <div class="grid grid-cols-2 gap-1.5">
            <Button v-for="d in [
              { k: 'backup', l: '备份导出' },
              { k: 'cache', l: '缓存' },
              { k: 'config', l: '配置' },
              { k: 'log', l: '日志' },
            ]" :key="d.k" variant="gray" size="sm" @click="openDir(d.k as never)">
              <FolderOpen class="h-3 w-3" /> {{ d.l }}
            </Button>
          </div>
        </Card>

        <Card class="p-4">
          <div class="mb-2 flex items-center justify-between">
            <p class="flex items-center gap-1.5 text-sm font-semibold">
              <ListChecks class="h-4 w-4 text-[var(--primary)]" /> 最近任务
            </p>
            <button type="button" class="text-[10px] text-[var(--primary)]" @click="route.go('tasks')">
              全部 →
            </button>
          </div>
          <p v-if="!recentTasks.length" class="text-xs text-[var(--muted)]">还没有任务，去软件库挑几个软件试试</p>
          <div v-else class="space-y-1">
            <div
              v-for="t in recentTasks"
              :key="t.id"
              class="flex items-center gap-2 rounded px-1 py-0.5 text-xs hover:bg-[var(--card-soft)]"
            >
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full"
                :style="{
                  background: t.status === 'done' ? '#30a46c' : t.status === 'failed' ? '#e5484d' : t.status === 'running' ? 'var(--primary)' : 'var(--muted)',
                }"
              />
              <span class="min-w-0 flex-1 truncate">{{ t.name }}</span>
              <span class="shrink-0 text-[10px] text-[var(--muted)]">{{ Math.round(t.progress) }}%</span>
            </div>
          </div>
        </Card>
      </div>
    </div>
  </div>
</template>
