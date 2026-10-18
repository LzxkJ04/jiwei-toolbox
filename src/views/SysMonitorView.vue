<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  Activity, ArrowDown, ArrowUp, Clock, Cpu, FolderOpen, Globe, Info, Loader2, MemoryStick,
  Network, RefreshCw, Search, Server, Trash2, X, Zap,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Input from "../components/ui/Input.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import Dialog from "../components/ui/Dialog.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import {
  dnsLookup, emptyRecycleBin, flushDns, killPid, killProcessTree, listPowerPlans, listProcesses, netAdapters,
  openPath, pingHost, setPowerPlan, sysUsage, uptimeInfo,
  type NetAdapter, type PingResult, type PowerPlan, type ProcRow, type SysUsage, type UptimeInfo,
} from "../lib/tauri";
import { copyText, formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

type Tab = "perf" | "process" | "network" | "maint";
const TABS: { id: Tab; label: string }[] = [
  { id: "perf", label: "实时性能" },
  { id: "process", label: "进程管理" },
  { id: "network", label: "网络诊断" },
  { id: "maint", label: "系统维护" },
];
const tab = ref<Tab>("perf");

/* ============ 实时曲线 ============ */

const POINTS = 60;
const cpuHistory = ref<number[]>([]);
const memHistory = ref<number[]>([]);
const downHistory = ref<number[]>([]);
const upHistory = ref<number[]>([]);
const usage = ref<SysUsage | null>(null);
const samples = ref(0);
const paused = ref(false);

const memPct = computed(() => {
  const u = usage.value;
  if (!u?.mem_total) return 0;
  return Math.min(100, (u.mem_used / u.mem_total) * 100);
});
const swapPct = computed(() => {
  const u = usage.value;
  if (!u?.swap_total) return 0;
  return Math.min(100, (u.swap_used / u.swap_total) * 100);
});
const maxNet = computed(() => Math.max(1, ...downHistory.value, ...upHistory.value));

/** 每核占用：超过 8 核时按两列折行，避免单行挤成一条线 */
const perCore = computed(() => usage.value?.per_core ?? []);

/** 折线图路径。values 已是 0-100 或原始值，按 max 归一化。 */
function linePath(values: number[], max = 100) {
  if (values.length < 2) return "";
  const w = 100;
  const h = 100;
  const step = w / (POINTS - 1);
  const offset = POINTS - values.length;
  return values
    .map((v, i) => {
      const x = (offset + i) * step;
      const y = h - Math.min(1, Math.max(0, v / max)) * h;
      return `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
}

function areaPath(values: number[], max = 100) {
  const line = linePath(values, max);
  if (!line) return "";
  const offset = POINTS - values.length;
  const step = 100 / (POINTS - 1);
  const lastX = ((offset + values.length - 1) * step).toFixed(2);
  const firstX = (offset * step).toFixed(2);
  return `${line} L${lastX},100 L${firstX},100 Z`;
}

function push(arr: { value: number[] }, v: number) {
  arr.value.push(v);
  if (arr.value.length > POINTS) arr.value.shift();
}

let busy = false;
async function tick() {
  if (busy) return;
  busy = true;
  try {
    const u = await sysUsage();
    if (u && u.mem_total > 0) {
      usage.value = u;
      samples.value += 1;
      if (!paused.value) {
        push(cpuHistory, u.cpu_pct);
        push(memHistory, (u.mem_used / u.mem_total) * 100);
        push(downHistory, u.down_bps);
        push(upHistory, u.up_bps);
      }
    }
  } finally {
    busy = false;
  }
}

/* ============ 进程 ============ */

const procs = ref<ProcRow[]>([]);
const procLoading = ref(false);
const procFilter = ref("");
const procSort = ref<"cpu" | "mem" | "name">("cpu");
const killing = ref<number | null>(null);

const shownProcs = computed(() => {
  const kw = procFilter.value.trim().toLowerCase();
  const list = kw
    ? procs.value.filter(
        (p) => p.name.toLowerCase().includes(kw) || p.exe.toLowerCase().includes(kw) || String(p.pid) === kw
      )
    : procs.value.slice();
  if (procSort.value === "mem") list.sort((a, b) => b.mem - a.mem);
  else if (procSort.value === "name") list.sort((a, b) => a.name.localeCompare(b.name));
  else list.sort((a, b) => b.cpu - a.cpu || b.mem - a.mem);
  return list.slice(0, 200);
});

async function loadProcs() {
  procLoading.value = true;
  const rows = await listProcesses();
  procs.value = rows ?? [];
  procLoading.value = false;
}

async function endProcess(p: ProcRow, tree = false) {
  if (killing.value !== null) return;
  killing.value = p.pid;
  try {
    if (tree) {
      const n = await killProcessTree(p.pid);
      toast(`已结束 ${p.name} 及其子进程，共 ${n} 个`, "success");
      // 子进程 PID 无法预知，结束后整表刷新一次
      await loadProcs();
    } else {
      await killPid(p.pid);
      toast(`已结束 ${p.name}`, "success");
      procs.value = procs.value.filter((x) => x.pid !== p.pid);
    }
  } catch (err) {
    toast(`结束失败：${String(err)}`, "error");
  } finally {
    killing.value = null;
  }
}

/* ---- 结束进程确认 ---- */
const pendingKill = ref<ProcRow | null>(null);
/** true = 连子进程一起结束 */
const killTree = ref(false);

function askKill(p: ProcRow, tree = false) {
  killTree.value = tree;
  pendingKill.value = p;
}

async function confirmKill() {
  const p = pendingKill.value;
  const tree = killTree.value;
  pendingKill.value = null;
  if (p) await endProcess(p, tree);
}

/* ---- 进程详情 ---- */
const detailProc = ref<ProcRow | null>(null);

async function copyProcInfo() {
  const p = detailProc.value;
  if (!p) return;
  const text = [
    `进程：${p.name}`,
    `PID：${p.pid}`,
    `CPU：${p.cpu.toFixed(1)}%`,
    `内存：${formatBytes(p.mem)}`,
    `路径：${p.exe || "（未取到）"}`,
    `命令行：${p.cmd || "（未取到）"}`,
  ].join("\n");
  const ok = await copyText(text);
  toast(ok ? "进程信息已复制" : "复制失败", ok ? "success" : "error");
}

async function openProcDir(p: ProcRow) {
  if (!p.exe) {
    toast("该进程没有可访问的路径", "info");
    return;
  }
  try {
    await openPath(p.exe);
  } catch (err) {
    toast(String(err), "error");
  }
}

/* ============ 网络诊断 ============ */

const adapters = ref<NetAdapter[]>([]);
const netLoading = ref(false);
const pingHostInput = ref("www.baidu.com");
const dnsInput = ref("www.baidu.com");
const pingResult = ref<PingResult | null>(null);
const pingLoading = ref(false);
const dnsResult = ref<string[] | null>(null);
const dnsLoading = ref(false);
const flushLoading = ref(false);

async function loadAdapters() {
  netLoading.value = true;
  adapters.value = (await netAdapters()) ?? [];
  netLoading.value = false;
}

async function doPing() {
  if (pingLoading.value) return;
  pingResult.value = null;
  pingLoading.value = true;
  try {
    pingResult.value = await pingHost(pingHostInput.value);
  } catch (err) {
    toast(`Ping 失败：${String(err)}`, "error");
  } finally {
    pingLoading.value = false;
  }
}

async function doDns() {
  if (dnsLoading.value) return;
  dnsResult.value = null;
  dnsLoading.value = true;
  try {
    dnsResult.value = await dnsLookup(dnsInput.value);
  } catch (err) {
    toast(`解析失败：${String(err)}`, "error");
  } finally {
    dnsLoading.value = false;
  }
}

async function doFlush() {
  flushLoading.value = true;
  try {
    const msg = await flushDns();
    toast(msg, "success");
  } catch (err) {
    toast(String(err), "error");
  } finally {
    flushLoading.value = false;
  }
}

function uptimeText(sec?: number) {
  if (!sec) return "—";
  const d = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  const m = Math.floor((sec % 3600) / 60);
  return [d ? `${d} 天` : "", h ? `${h} 小时` : "", `${m} 分`].filter(Boolean).join(" ");
}

function speedText(bps: number) {
  return `${formatBytes(bps, 1)}/s`;
}

function barColor(pct: number) {
  if (pct >= 90) return "#e5484d";
  if (pct >= 75) return "#f5a623";
  return "var(--primary)";
}

/* ============ 系统维护 ============ */

const plans = ref<PowerPlan[]>([]);
const plansLoading = ref(false);
const planBusy = ref(false);
const uptime = ref<UptimeInfo | null>(null);
const binBusy = ref(false);
const showEmptyBin = ref(false);

async function loadMaint() {
  plansLoading.value = true;
  const [p, u] = await Promise.all([listPowerPlans(), uptimeInfo()]);
  plans.value = p ?? [];
  uptime.value = u;
  plansLoading.value = false;
}

async function switchPlan(id: string) {
  if (planBusy.value) return;
  planBusy.value = true;
  try {
    await setPowerPlan(id);
    plans.value = plans.value.map((p) => ({ ...p, active: p.id === id }));
    const name = plans.value.find((p) => p.id === id)?.name ?? "";
    toast(`已切换到「${name}」`, "success");
  } catch (err) {
    toast(String(err), "error");
  } finally {
    planBusy.value = false;
  }
}

async function doEmptyBin() {
  showEmptyBin.value = false;
  binBusy.value = true;
  try {
    await emptyRecycleBin();
    toast("回收站已清空", "success");
  } catch (err) {
    toast(String(err), "error");
  } finally {
    binBusy.value = false;
  }
}

const bootDate = computed(() => {
  const ts = uptime.value?.boot_ts;
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
});

/* ============ 生命周期 ============ */

let timer: number | undefined;

onMounted(() => {
  void tick();
  void loadMaint();
  timer = window.setInterval(() => {
    if (document.visibilityState === "visible") void tick();
  }, 1_500);
});

onUnmounted(() => {
  if (timer !== undefined) window.clearInterval(timer);
});
</script>

<template>
  <div>
    <PageHeader title="系统监控" desc="占用曲线、进程管理、网络诊断。轮询只在页面可见时进行。">
      <template #extra>
        <Button variant="gray" size="sm" @click="paused = !paused">
          {{ paused ? "继续曲线" : "暂停曲线" }}
        </Button>
      </template>
    </PageHeader>

    <div class="mb-4 flex flex-wrap gap-1 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-1">
      <button
        v-for="t in TABS"
        :key="t.id"
        type="button"
        class="rounded-lg px-3 py-1.5 text-xs font-medium transition-colors"
        :class="tab === t.id ? 'bg-[var(--primary)] text-white' : 'text-[var(--muted)] hover:bg-[var(--btn-gray)] hover:text-[var(--text)]'"
        @click="tab = t.id"
      >
        {{ t.label }}
      </button>
    </div>

    <!-- 实时性能 -->
    <div v-show="tab === 'perf'" class="space-y-4">
      <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <p class="flex items-center gap-1 text-[10px] text-[var(--muted)]"><Cpu class="h-3 w-3" /> CPU</p>
          <p class="mt-1 text-2xl font-bold tabular-nums text-[var(--text-strong)]">
            {{ samples ? `${(usage?.cpu_pct ?? 0).toFixed(0)}%` : "…" }}
          </p>
        </div>
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <p class="flex items-center gap-1 text-[10px] text-[var(--muted)]"><MemoryStick class="h-3 w-3" /> 内存</p>
          <p class="mt-1 text-2xl font-bold tabular-nums text-[var(--text-strong)]">{{ memPct.toFixed(0) }}%</p>
          <p class="truncate text-[10px] text-[var(--muted)]">
            {{ formatBytes(usage?.mem_used ?? 0) }} / {{ formatBytes(usage?.mem_total ?? 0) }}
          </p>
        </div>
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <p class="flex items-center gap-1 text-[10px] text-[var(--muted)]"><ArrowDown class="h-3 w-3" /> 下载</p>
          <p class="mt-1 text-lg font-bold tabular-nums text-[var(--text-strong)]">{{ speedText(usage?.down_bps ?? 0) }}</p>
          <p class="flex items-center gap-1 text-[10px] text-[var(--muted)]">
            <ArrowUp class="h-3 w-3" /> {{ speedText(usage?.up_bps ?? 0) }}
          </p>
        </div>
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <p class="flex items-center gap-1 text-[10px] text-[var(--muted)]"><Activity class="h-3 w-3" /> 进程</p>
          <p class="mt-1 text-2xl font-bold tabular-nums text-[var(--text-strong)]">{{ usage?.processes ?? "—" }}</p>
          <p class="flex items-center gap-1 truncate text-[10px] text-[var(--muted)]">
            <Clock class="h-3 w-3 shrink-0" /> 已运行 {{ uptimeText(usage?.uptime) }}
          </p>
        </div>
      </div>

      <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card class="p-4">
          <div class="mb-2 flex items-center justify-between">
            <p class="text-sm font-semibold">CPU 占用</p>
            <span class="text-xs tabular-nums text-[var(--muted)]">{{ (usage?.cpu_pct ?? 0).toFixed(1) }}%</span>
          </div>
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="h-32 w-full overflow-visible">
            <path :d="areaPath(cpuHistory)" fill="var(--primary)" opacity="0.14" />
            <path :d="linePath(cpuHistory)" fill="none" stroke="var(--primary)" stroke-width="1.6" vector-effect="non-scaling-stroke" />
          </svg>
        </Card>
        <Card class="p-4">
          <div class="mb-2 flex items-center justify-between">
            <p class="text-sm font-semibold">内存占用</p>
            <span class="text-xs tabular-nums text-[var(--muted)]">{{ memPct.toFixed(1) }}%</span>
          </div>
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="h-32 w-full overflow-visible">
            <path :d="areaPath(memHistory)" fill="#30a46c" opacity="0.14" />
            <path :d="linePath(memHistory)" fill="none" stroke="#30a46c" stroke-width="1.6" vector-effect="non-scaling-stroke" />
          </svg>
        </Card>
        <Card class="p-4 lg:col-span-2">
          <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
            <p class="text-sm font-semibold">网络速率</p>
            <div class="flex items-center gap-3 text-xs text-[var(--muted)]">
              <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-[#3b82f6]" /> 下载</span>
              <span class="flex items-center gap-1"><span class="h-2 w-2 rounded-full bg-[#a78bfa]" /> 上传</span>
              <span class="tabular-nums">峰值 {{ speedText(maxNet) }}</span>
            </div>
          </div>
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="h-36 w-full overflow-visible">
            <path :d="areaPath(downHistory, maxNet)" fill="#3b82f6" opacity="0.12" />
            <path :d="linePath(downHistory, maxNet)" fill="none" stroke="#3b82f6" stroke-width="1.6" vector-effect="non-scaling-stroke" />
            <path :d="linePath(upHistory, maxNet)" fill="none" stroke="#a78bfa" stroke-width="1.4" stroke-dasharray="3 2" vector-effect="non-scaling-stroke" />
          </svg>
        </Card>
      </div>

      <div v-if="usage?.swap_total" class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
        <div class="mb-1 flex items-baseline justify-between text-xs">
          <span class="text-[var(--muted)]">虚拟内存</span>
          <span class="tabular-nums text-[var(--text)]">
            {{ formatBytes(usage.swap_used) }} / {{ formatBytes(usage.swap_total) }}
          </span>
        </div>
        <div class="h-1.5 overflow-hidden rounded-full bg-[var(--btn-gray)]">
          <div class="h-full rounded-full" :style="{ width: `${swapPct}%`, background: barColor(swapPct) }" />
        </div>
      </div>

      <!-- 每核占用 -->
      <Card v-if="perCore.length" class="p-4">
        <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
          <p class="text-sm font-semibold">各核心占用</p>
          <span class="text-xs text-[var(--muted)]">{{ perCore.length }} 个逻辑核心</span>
        </div>
        <div class="grid grid-cols-2 gap-x-4 gap-y-1.5 sm:grid-cols-3 lg:grid-cols-4">
          <div v-for="(v, i) in perCore" :key="i" class="flex items-center gap-2">
            <span class="w-9 shrink-0 text-right text-[10px] tabular-nums text-[var(--muted)]">#{{ i }}</span>
            <div class="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-[var(--btn-gray)]">
              <div
                class="h-full rounded-full transition-[width] duration-300"
                :style="{ width: `${Math.min(100, v)}%`, background: barColor(v) }"
              />
            </div>
            <span class="w-8 shrink-0 text-right text-[10px] tabular-nums text-[var(--muted)]">{{ v.toFixed(0) }}%</span>
          </div>
        </div>
        <p class="mt-2 text-[10px] text-[var(--muted)]">
          单核打满时总占用可能不高，这里能看出是不是某个核心独占。
        </p>
      </Card>

      <p class="text-[10px] text-[var(--muted)]">每 1.5 秒采样一次，保留最近 60 个点（约 90 秒）。页面切走时暂停采集。</p>
    </div>

    <!-- 进程管理 -->
    <div v-show="tab === 'process'" class="space-y-3">
      <div class="flex flex-wrap items-center gap-2">
        <div class="relative min-w-[200px] flex-1">
          <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
          <Input v-model="procFilter" placeholder="按进程名、路径或 PID 搜索" class="!pl-9" />
        </div>
        <div class="flex gap-1 rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] p-1">
          <button
            v-for="s in [{ v: 'cpu', l: 'CPU' }, { v: 'mem', l: '内存' }, { v: 'name', l: '名称' }]"
            :key="s.v"
            type="button"
            class="rounded px-2 py-1 text-xs"
            :class="procSort === s.v ? 'bg-[var(--primary)] text-white' : 'text-[var(--muted)]'"
            @click="procSort = s.v as never"
          >
            {{ s.l }}
          </button>
        </div>
        <Button variant="gray" size="sm" :disabled="procLoading" @click="loadProcs">
          <RefreshCw class="h-3.5 w-3.5" :class="procLoading && 'animate-spin'" />
          {{ procs.length ? "刷新" : "读取进程" }}
        </Button>
      </div>

      <Spinner v-if="procLoading" label="正在采样进程…" />
      <EmptyState
        v-else-if="!procs.length"
        icon="🧩"
        text="还没有进程数据"
        sub="点右上角「读取进程」开始采样。CPU 占用要两次采样才有值，首次会稍慢。"
      />
      <Card v-else class="p-0">
        <div class="overflow-x-auto">
          <table class="w-full min-w-[720px] text-left text-xs">
            <thead>
              <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
                <th class="px-3 py-2.5 font-medium">进程</th>
                <th class="px-3 py-2.5 text-right font-medium">PID</th>
                <th class="px-3 py-2.5 text-right font-medium">CPU</th>
                <th class="px-3 py-2.5 text-right font-medium">内存</th>
                <th class="px-3 py-2.5 font-medium">路径</th>
                <th class="px-3 py-2.5 text-right font-medium">操作</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-[var(--card-border)]">
              <tr v-for="p in shownProcs" :key="p.pid" class="transition-colors hover:bg-[var(--card-soft)]">
                <td class="max-w-[160px] truncate px-3 py-2 font-medium text-[var(--text-strong)]" :title="p.name">
                  {{ p.name || "—" }}
                </td>
                <td class="px-3 py-2 text-right font-mono tabular-nums">{{ p.pid }}</td>
                <td class="px-3 py-2 text-right tabular-nums" :class="p.cpu >= 25 ? 'font-semibold text-[#f5a623]' : ''">
                  {{ p.cpu.toFixed(1) }}%
                </td>
                <td class="px-3 py-2 text-right tabular-nums">{{ formatBytes(p.mem) }}</td>
                <td class="max-w-[280px] truncate px-3 py-2 text-[var(--muted)]" :title="p.exe || p.cmd">
                  {{ p.exe || p.cmd || "—" }}
                </td>
                <td class="whitespace-nowrap px-3 py-2 text-right">
                  <Button variant="gray" size="sm" title="查看详情" @click="detailProc = p">
                    <Info class="h-3 w-3" />
                  </Button>
                  <Button
                    variant="gray"
                    size="sm"
                    class="ml-1"
                    :disabled="killing !== null"
                    title="结束进程"
                    @click="askKill(p)"
                  >
                    <Loader2 v-if="killing === p.pid" class="h-3 w-3 animate-spin" />
                    <X v-else class="h-3 w-3" />
                    结束
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    :disabled="killing !== null"
                    title="结束该进程及其全部子进程"
                    @click="askKill(p, true)"
                  >
                    含子进程
                  </Button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </Card>
      <p v-if="procs.length" class="text-[10px] text-[var(--muted)]">
        共采样 {{ procs.length }} 个进程，列表最多显示 200 行。结束系统关键进程会失败并提示原因。
      </p>
    </div>

    <!-- 网络诊断 -->
    <div v-show="tab === 'network'" class="space-y-4">
      <Card class="p-4">
        <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
          <p class="flex items-center gap-2 text-sm font-semibold"><Network class="h-4 w-4 text-[var(--primary)]" /> 网络适配器</p>
          <Button variant="gray" size="sm" :disabled="netLoading" @click="loadAdapters">
            <RefreshCw class="h-3.5 w-3.5" :class="netLoading && 'animate-spin'" />
            {{ adapters.length ? "刷新" : "读取" }}
          </Button>
        </div>
        <Spinner v-if="netLoading" label="正在读取适配器…" />
        <p v-else-if="!adapters.length" class="text-xs text-[var(--muted)]">点「读取」获取当前已连接的网卡信息</p>
        <div v-else class="space-y-2">
          <div v-for="a in adapters" :key="a.alias" class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
            <div class="flex flex-wrap items-center gap-2">
              <p class="text-sm font-semibold text-[var(--text-strong)]">{{ a.alias }}</p>
              <Badge v-if="a.speed" tone="primary">{{ a.speed }}</Badge>
            </div>
            <div class="mt-1 grid grid-cols-1 gap-x-6 gap-y-0.5 text-xs text-[var(--muted)] sm:grid-cols-2">
              <p class="truncate">IP {{ a.ipv4 || "—" }}</p>
              <p class="truncate">网关 {{ a.gateway || "—" }}</p>
              <p class="truncate">DNS {{ a.dns || "—" }}</p>
              <p class="truncate">MAC {{ a.mac || "—" }}</p>
            </div>
          </div>
        </div>
      </Card>

      <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card class="p-4">
          <p class="mb-3 flex items-center gap-2 text-sm font-semibold"><Zap class="h-4 w-4 text-[var(--primary)]" /> 连通性测试</p>
          <div class="flex gap-2">
            <Input v-model="pingHostInput" placeholder="域名或 IP，如 www.baidu.com" class="flex-1" />
            <Button variant="primary" size="sm" :disabled="pingLoading" @click="doPing">
              <Loader2 v-if="pingLoading" class="h-3.5 w-3.5 animate-spin" />
              <Zap v-else class="h-3.5 w-3.5" />
              Ping
            </Button>
          </div>
          <div v-if="pingResult" class="mt-3 grid grid-cols-2 gap-2 text-xs sm:grid-cols-4">
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">丢包</p>
              <p class="font-bold tabular-nums" :class="pingResult.loss_pct > 0 ? 'text-[#e5484d]' : 'text-[#30a46c]'">
                {{ pingResult.loss_pct }}%
              </p>
            </div>
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">最小</p>
              <p class="font-bold tabular-nums">{{ pingResult.min ?? "—" }}<span class="text-[10px]"> ms</span></p>
            </div>
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">平均</p>
              <p class="font-bold tabular-nums">{{ pingResult.avg ?? "—" }}<span class="text-[10px]"> ms</span></p>
            </div>
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">最大</p>
              <p class="font-bold tabular-nums">{{ pingResult.max ?? "—" }}<span class="text-[10px]"> ms</span></p>
            </div>
          </div>
          <p class="mt-2 text-[10px] text-[var(--muted)]">发 4 个 ICMP 包。部分网络会屏蔽 Ping，丢包 100% 不等于断网。</p>
        </Card>

        <Card class="p-4">
          <p class="mb-3 flex items-center gap-2 text-sm font-semibold"><Globe class="h-4 w-4 text-[var(--primary)]" /> 域名解析</p>
          <div class="flex gap-2">
            <Input v-model="dnsInput" placeholder="要解析的域名" class="flex-1" />
            <Button variant="primary" size="sm" :disabled="dnsLoading" @click="doDns">
              <Loader2 v-if="dnsLoading" class="h-3.5 w-3.5 animate-spin" />
              <Server v-else class="h-3.5 w-3.5" />
              解析
            </Button>
          </div>
          <div v-if="dnsResult" class="mt-3 space-y-1">
            <p v-for="ip in dnsResult" :key="ip" class="rounded-lg bg-[var(--card-soft)] px-2 py-1 font-mono text-xs">{{ ip }}</p>
          </div>
          <div class="mt-3 border-t border-[var(--card-border)] pt-3">
            <Button variant="gray" size="sm" :disabled="flushLoading" @click="doFlush">
              <Loader2 v-if="flushLoading" class="h-3.5 w-3.5 animate-spin" />
              <Trash2 v-else class="h-3.5 w-3.5" />
              清理 DNS 缓存
            </Button>
          </div>
        </Card>
      </div>
    </div>

    <!-- 系统维护 -->
    <div v-show="tab === 'maint'" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <Card class="p-4">
        <p class="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Zap class="h-4 w-4 text-[var(--primary)]" /> 电源计划
        </p>
        <Spinner v-if="plansLoading" label="正在读取电源计划…" />
        <p v-else-if="!plans.length" class="text-xs text-[var(--muted)]">没有读到电源计划</p>
        <div v-else class="space-y-2">
          <button
            v-for="p in plans"
            :key="p.id"
            type="button"
            :disabled="planBusy || p.active"
            class="flex w-full items-center justify-between gap-2 rounded-lg border px-3 py-2 text-left text-sm transition-colors"
            :class="p.active
              ? 'border-[var(--primary)] bg-[var(--primary-soft)] text-[var(--primary)]'
              : 'border-[var(--card-border)] bg-[var(--card-soft)] hover:border-[var(--primary)]'"
            @click="switchPlan(p.id)"
          >
            <span class="truncate">{{ p.name }}</span>
            <Badge v-if="p.active" tone="primary">使用中</Badge>
          </button>
        </div>
        <p class="mt-2 text-[10px] text-[var(--muted)]">切换电源计划会影响 CPU 性能与待机功耗，台式机建议「高性能」。</p>
      </Card>

      <div class="space-y-4">
        <Card class="p-4">
          <p class="mb-3 flex items-center gap-2 text-sm font-semibold">
            <Clock class="h-4 w-4 text-[var(--primary)]" /> 运行时长
          </p>
          <div class="space-y-2">
            <div class="flex items-baseline justify-between border-b border-[var(--card-border)] py-1.5 text-xs">
              <span class="text-[var(--muted)]">本次开机时间</span>
              <span class="font-medium">{{ bootDate }}</span>
            </div>
            <div class="flex items-baseline justify-between border-b border-[var(--card-border)] py-1.5 text-xs">
              <span class="text-[var(--muted)]">已运行</span>
              <span class="font-medium">{{ uptimeText(uptime?.uptime) }}</span>
            </div>
            <div v-if="uptime?.boot_seconds" class="flex items-baseline justify-between py-1.5 text-xs">
              <span class="text-[var(--muted)]">上次启动耗时</span>
              <span class="font-medium">{{ uptime.boot_seconds }} 秒</span>
            </div>
          </div>
        </Card>

        <Card class="p-4">
          <p class="mb-3 flex items-center gap-2 text-sm font-semibold">
            <Trash2 class="h-4 w-4 text-[var(--primary)]" /> 回收站
          </p>
          <Button variant="gray" size="sm" :disabled="binBusy" @click="showEmptyBin = true">
            <Loader2 v-if="binBusy" class="h-3.5 w-3.5 animate-spin" />
            <Trash2 v-else class="h-3.5 w-3.5" />
            清空回收站
          </Button>
          <p class="mt-2 text-[10px] text-[var(--muted)]">永久删除回收站内的全部内容，不可恢复。</p>
        </Card>
      </div>
    </div>

    <!-- 进程详情 -->
    <Dialog :open="!!detailProc" title="进程详情" @update:open="(v) => !v && (detailProc = null)">
      <div v-if="detailProc" class="space-y-2">
        <div v-for="[k, v] in [
          ['进程名', detailProc.name || '—'],
          ['PID', String(detailProc.pid)],
          ['CPU 占用', `${detailProc.cpu.toFixed(1)}%`],
          ['内存占用', formatBytes(detailProc.mem)],
        ]" :key="k" class="flex items-baseline justify-between gap-3 border-b border-[var(--card-border)] py-1.5">
          <span class="shrink-0 text-xs text-[var(--muted)]">{{ k }}</span>
          <span class="min-w-0 truncate text-sm font-medium">{{ v }}</span>
        </div>
        <div>
          <p class="mb-1 text-xs text-[var(--muted)]">可执行文件路径</p>
          <p class="break-all rounded-lg bg-[var(--card-soft)] p-2 font-mono text-xs">{{ detailProc.exe || "（未取到）" }}</p>
        </div>
        <div v-if="detailProc.cmd && detailProc.cmd !== detailProc.exe">
          <p class="mb-1 text-xs text-[var(--muted)]">命令行</p>
          <p class="break-all rounded-lg bg-[var(--card-soft)] p-2 font-mono text-xs">{{ detailProc.cmd }}</p>
        </div>
      </div>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-2">
          <Button variant="gray" @click="detailProc && openProcDir(detailProc)">
            <FolderOpen class="h-4 w-4" /> 打开所在目录
          </Button>
          <Button variant="gray" @click="copyProcInfo">复制信息</Button>
          <Button v-if="detailProc" variant="danger" @click="askKill(detailProc); detailProc = null">结束进程</Button>
        </div>
      </template>
    </Dialog>

    <ConfirmDialog
      :open="showEmptyBin"
      title="清空回收站？"
      message="回收站里的所有文件都会被永久删除，无法恢复。"
      confirm-text="清空"
      danger
      @update:open="showEmptyBin = $event"
      @confirm="doEmptyBin"
    />

    <ConfirmDialog
      :open="!!pendingKill"
      title="结束这个进程？"
      :message="pendingKill
        ? (killTree
          ? `将结束 ${pendingKill.name}（PID ${pendingKill.pid}）以及它的全部子进程。子进程数量未知，未保存的数据可能丢失。`
          : `将强制结束 ${pendingKill.name}（PID ${pendingKill.pid}）。未保存的数据可能丢失。`)
        : ''"
      :confirm-text="killTree ? '结束进程树' : '结束进程'"
      danger
      @update:open="(v) => !v && (pendingKill = null)"
      @confirm="confirmKill"
    />
  </div>
</template>
