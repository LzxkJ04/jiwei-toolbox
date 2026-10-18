<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  CircuitBoard, Copy, Cpu, Eye, EyeOff, Flame, Grid3x3, HardDrive, Loader2, MemoryStick, Monitor,
  Play, RefreshCw, Square, Zap,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Progress from "../components/ui/Progress.vue";
import Select from "../components/ui/Select.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import {
  cpuBurnStart, cpuBurnStop, cpuTemp, diskSmart, diskSpeed, gpuBurnStart, gpuBurnStop,
  gpuTemp, hwInfo, hwUsage, memTestStart, onCpuBurnEnd, onCpuLoad, onGpuLoad, onMemTestDone, onMemTestProgress,
  type DiskSmartInfo, type DiskSpeed, type HwDisk, type HwGpu, type HwInfo, type HwMemoryModule, type HwUsage, type MemTestResult,
} from "../lib/tauri";
import { copyText, formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

type HwTab = "overview" | "cpu" | "board" | "gpu" | "disk" | "stress";

const TABS: { id: HwTab; label: string }[] = [
  { id: "overview", label: "总览" },
  { id: "cpu", label: "处理器" },
  { id: "board", label: "内存主板" },
  { id: "gpu", label: "显卡" },
  { id: "disk", label: "存储" },
  { id: "stress", label: "压测" },
];

const tab = ref<HwTab>("overview");
const hideVirtual = ref(true);

const BURN_OPTS = [
  { value: "30", label: "30 秒" },
  { value: "60", label: "60 秒" },
  { value: "300", label: "5 分钟" },
];

function fmtCountdown(totalSec: number) {
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function dash(v?: string | number | null) {
  if (v === null || v === undefined) return "—";
  const s = String(v).trim();
  return s ? s : "—";
}

function clockText(mhz?: number) {
  if (!mhz) return "—";
  if (mhz >= 1000) return `${(mhz / 1000).toFixed(2)} GHz`;
  return `${mhz} MHz`;
}

function cacheText(kb?: number) {
  if (!kb) return "—";
  if (kb >= 1024) {
    const mb = kb / 1024;
    return `${Number.isInteger(mb) ? mb.toFixed(0) : mb.toFixed(1)} MB`;
  }
  return `${kb} KB`;
}

function virtText(v: boolean | null | undefined) {
  if (v === true) return "已开启";
  if (v === false) return "未开启";
  return "未报告";
}

/* ============ 硬件信息 ============ */

const hw = ref<HwInfo | null>(null);
const hwLoading = ref(false);
const usage = ref<HwUsage | null>(null);
const usageSamples = ref(0);

const visibleGpus = computed(() => {
  const list = hw.value?.gpus || [];
  return hideVirtual.value ? list.filter((g) => !g.virtual) : list;
});
const hiddenGpuCount = computed(() => (hw.value?.gpus || []).filter((g) => g.virtual).length);
const primaryGpu = computed(() => visibleGpus.value[0] || hw.value?.gpus?.[0]);

const osLine = computed(() => {
  const os = hw.value?.os;
  if (!os) return "";
  return [os.caption, os.version, os.build ? `Build ${os.build}` : "", os.arch].filter(Boolean).join(" · ");
});

const cpuLive = computed(() => (usageSamples.value >= 2 ? usage.value?.cpu_pct ?? null : null));
const memPct = computed(() => {
  const u = usage.value;
  if (!u?.mem_total) return 0;
  return Math.min(100, Math.max(0, (u.mem_used / u.mem_total) * 100));
});

function cpuSub(cpu?: HwInfo["cpu"]) {
  const base = `${cpu?.cores || 0} 核 / ${cpu?.threads || 0} 线程`;
  const parts = [base];
  if (cpu?.arch) parts.push(cpu.arch);
  if (cpu?.mhz) parts.push(`最高 ${clockText(cpu.mhz)}`);
  if (cpu?.current_mhz) parts.push(`当前 ${clockText(cpu.current_mhz)}`);
  return parts.join(" · ");
}

function vramText(vram: number) {
  if (!vram || vram <= 0) return "";
  const mb = vram > 65536 ? Math.round(vram / 1048576) : Math.round(vram);
  if (mb <= 0) return "";
  if (mb >= 1024) {
    const gb = mb / 1024;
    return `${Number.isInteger(gb) ? gb.toFixed(0) : gb.toFixed(1)} GB`;
  }
  return `${mb} MB`;
}

function gpuSub(g: HwGpu) {
  const parts: string[] = [];
  if (g.virtual) parts.push("虚拟显示器");
  const vram = vramText(g.vram || 0);
  if (vram) parts.push(`${vram} 显存`);
  else if (!g.virtual) parts.push("显存未报告");
  if (g.driver) parts.push(`驱动 ${g.driver}`);
  return parts.join(" · ");
}

function resText(g: HwGpu) {
  if (!g.width || !g.height) return "—";
  return `${g.width}×${g.height}${g.refresh ? ` @ ${g.refresh} Hz` : ""}`;
}

function diskSub(d: HwDisk) {
  const parts: string[] = [];
  if (d.size && d.size > 0) parts.push(formatBytes(d.size));
  if (d.bus && d.bus !== "SCSI") parts.push(d.bus);
  if (d.media) parts.push(d.media);
  if (d.health) parts.push(healthMeta(d.health).label);
  return parts.join(" · ") || "容量未知";
}

function moduleBrief(m: HwMemoryModule) {
  return [m.manufacturer, m.capacity_gb ? `${m.capacity_gb} GB` : "", m.type, m.speed ? `${m.speed} MHz` : ""]
    .filter(Boolean)
    .join(" · ");
}

function usedPct(total?: number, free?: number) {
  if (!total) return 0;
  return Math.min(100, Math.max(0, ((total - (free || 0)) / total) * 100));
}

function barColor(pct: number) {
  if (pct >= 90) return "#e5484d";
  if (pct >= 75) return "#f5a623";
  return "var(--primary)";
}

const cpuRows = computed(() => {
  const c = hw.value?.cpu;
  return [
    ["型号", dash(c?.name)],
    ["厂商", dash(c?.vendor)],
    ["架构", dash(c?.arch)],
    ["插槽", dash(c?.socket)],
    ["核心 / 线程", c ? `${c.cores || 0} / ${c.threads || 0}` : "—"],
    ["最高频率", clockText(c?.mhz)],
    ["当前频率", clockText(c?.current_mhz)],
    ["二级缓存", cacheText(c?.l2_kb)],
    ["三级缓存", cacheText(c?.l3_kb)],
    ["硬件虚拟化", virtText(c?.virtualization)],
  ] as [string, string][];
});

const boardRows = computed(() => {
  const b = hw.value?.motherboard;
  const bios = hw.value?.bios;
  return [
    ["主板厂商", dash(b?.manufacturer)],
    ["主板型号", dash(b?.product)],
    ["主板版本", dash(b?.version)],
    ["主板序列号", dash(b?.serial)],
    ["BIOS 厂商", dash(bios?.vendor)],
    ["BIOS 版本", dash(bios?.version)],
    ["BIOS 日期", dash(bios?.date)],
    ["系统", dash(osLine.value)],
  ] as [string, string][];
});

async function loadHw() {
  hwLoading.value = true;
  hw.value = await hwInfo();
  hwLoading.value = false;
}

async function copyReport() {
  const h = hw.value;
  if (!h) return;
  const lines = [
    "极维工具箱 · 硬件报告",
    osLine.value ? `系统：${osLine.value}` : "",
    `处理器：${h.cpu.name || "—"}`,
    `  ${cpuSub(h.cpu)}`,
    `  厂商 ${dash(h.cpu.vendor)} · 插槽 ${dash(h.cpu.socket)} · L2 ${cacheText(h.cpu.l2_kb)} · L3 ${cacheText(h.cpu.l3_kb)} · 虚拟化 ${virtText(h.cpu.virtualization)}`,
    `内存：${formatBytes(h.memory_total)}`,
    ...(h.memoryModules || []).map((m) => `  ${moduleBrief(m)} · ${dash(m.part_number)} · ${dash(m.form_factor)} · ${dash(m.slot)}`),
    `主板：${h.board || "—"}`,
    `  序列号 ${dash(h.motherboard?.serial)} · BIOS ${dash(h.bios?.vendor)} ${dash(h.bios?.version)} ${dash(h.bios?.date)}`,
    "显卡：",
    ...(h.gpus || []).map((g) => `  ${g.name} · ${gpuSub(g)} · ${resText(g)} · 驱动日期 ${dash(g.driver_date)}`),
    "磁盘：",
    ...(h.disks || []).map((d) => `  ${d.model} · ${diskSub(d)} · SN ${dash(d.serial)} · FW ${dash(d.firmware)}`),
    ...(h.volumes || []).map((v) => `  ${v.mount} 可用 ${formatBytes(v.free)} / ${formatBytes(v.total)}`),
  ].filter(Boolean);
  const ok = await copyText(lines.join("\n"));
  toast(ok ? "硬件报告已复制" : "复制失败", ok ? "success" : "error");
}

/* ============ CPU 烤机 ============ */

const cpuDur = ref("60");
const cpuRunning = ref(false);
const cpuPct = ref(0);
/** 压测中的实时温度与峰值，超温保护用 */
const burnTemp = ref<number | null>(null);
const burnPeak = ref(0);
const cpuRemain = computed(() => Math.max(0, Math.ceil((Number(cpuDur.value) * (100 - cpuPct.value)) / 100)));

async function cpuStart() {
  if (cpuRunning.value) return;
  cpuPct.value = 0;
  burnTemp.value = null;
  burnPeak.value = 0;
  cpuRunning.value = true;
  await cpuBurnStart(Number(cpuDur.value));
}

async function cpuStop() {
  await cpuBurnStop();
  cpuRunning.value = false;
  cpuPct.value = 0;
}

/* ============ GPU 烤机 ============ */

const gpuDur = ref("60");
const gpuRunning = ref(false);
const gpuPct = ref(0);
const gpuFps = ref(0);
const gpuError = ref<string | null>(null);
const gpuRemain = computed(() => Math.max(0, Math.ceil((Number(gpuDur.value) * (100 - gpuPct.value)) / 100)));

async function gpuStart() {
  if (gpuRunning.value || gpuError.value) return;
  gpuPct.value = 0;
  gpuFps.value = 0;
  gpuRunning.value = true;
  await gpuBurnStart(Number(gpuDur.value));
}

async function gpuStop() {
  await gpuBurnStop();
  gpuRunning.value = false;
  gpuPct.value = 0;
  gpuFps.value = 0;
}

/* ============ 内存测试 ============ */

const memRunning = ref(false);
const memPctTest = ref(0);
const memErrors = ref(0);
const memResult = ref<MemTestResult | null>(null);

async function memStart() {
  if (memRunning.value) return;
  memResult.value = null;
  memPctTest.value = 0;
  memErrors.value = 0;
  memRunning.value = true;
  await memTestStart();
}

/* ============ 磁盘测速 ============ */

const FALLBACK_DRIVES = ["C", "D", "E", "F", "G", "H"].map((d) => ({ value: `${d}:\\`, label: `${d} 盘` }));
const diskSel = ref("C:\\");
const diskTesting = ref(false);
const diskRes = ref<DiskSpeed | null>(null);

const driveOpts = computed(() => {
  const seen = new Set<string>();
  const opts = (hw.value?.volumes || [])
    .map((v) => {
      const letter = v.mount.match(/([A-Za-z]):/)?.[1]?.toUpperCase();
      if (!letter || seen.has(letter)) return null;
      seen.add(letter);
      return { value: `${letter}:\\`, label: `${letter} 盘` };
    })
    .filter((x): x is { value: string; label: string } => !!x);
  return opts.length ? opts : FALLBACK_DRIVES;
});

async function runDiskSpeed() {
  if (diskTesting.value) return;
  diskTesting.value = true;
  diskRes.value = null;
  diskRes.value = await diskSpeed(diskSel.value);
  diskTesting.value = false;
}

function fmtMbps(n: number | null | undefined) {
  return typeof n === "number" && Number.isFinite(n) ? n.toFixed(1) : "—";
}

/* ============ 坏点检测 ============ */

const PIXEL_COLORS = [
  { label: "红", value: "#ff0000" },
  { label: "绿", value: "#00ff00" },
  { label: "蓝", value: "#0000ff" },
  { label: "白", value: "#ffffff" },
  { label: "黑", value: "#000000" },
];
const pixelColor = ref<string | null>(null);

function closePixel() {
  pixelColor.value = null;
}

function onGlobalKey(e: KeyboardEvent) {
  if (e.key === "Escape") closePixel();
}

/* ============ 温度 + 占用 ============ */

const cpuT = ref<number | null>(null);
const gpuT = ref<number | null>(null);
const tempLoading = ref(false);

async function refreshTemps() {
  tempLoading.value = true;
  const [c, g] = await Promise.all([cpuTemp(), gpuTemp()]);
  cpuT.value = c;
  gpuT.value = g;
  tempLoading.value = false;
}

let usageBusy = false;
async function refreshUsage() {
  if (usageBusy) return;
  usageBusy = true;
  try {
    const u = await hwUsage();
    if (u && u.mem_total > 0) {
      usage.value = u;
      usageSamples.value += 1;
    }
  } finally {
    usageBusy = false;
  }
}

let tempTimer: number | undefined;
let usageTimer: number | undefined;

function startPolling() {
  if (tempTimer === undefined) {
    tempTimer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refreshTemps();
    }, 10_000);
  }
  if (usageTimer === undefined) {
    usageTimer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refreshUsage();
    }, 2_000);
  }
}

function stopPolling() {
  if (tempTimer !== undefined) {
    window.clearInterval(tempTimer);
    tempTimer = undefined;
  }
  if (usageTimer !== undefined) {
    window.clearInterval(usageTimer);
    usageTimer = undefined;
  }
}

function onVisibilityChange() {
  if (document.visibilityState === "visible") {
    void refreshTemps();
    void refreshUsage();
  }
}

function tempClass(t: number | null) {
  if (t === null) return "text-[var(--muted)]";
  if (t >= 80) return "text-[#e5484d]";
  if (t >= 60) return "text-[#f5a623]";
  return "text-[#30a46c]";
}
function tempText(t: number | null) {
  return t === null ? "—" : `${t}℃`;
}

/* ============ 温度传感器 ============ */

interface HwSensor {
  label: string;
  temperature: number;
  max?: number | null;
}

const sensors = ref<HwSensor[]>([]);
const sensorsLoading = ref(false);
const sensorsRead = ref(false);
const sensorsError = ref("");

function sensorTempText(t: number | null | undefined) {
  if (t == null || !Number.isFinite(t)) return "—";
  return `${t}℃`;
}

async function readSensors() {
  if (sensorsLoading.value) return;
  sensorsLoading.value = true;
  sensorsError.value = "";
  try {
    const list = await invoke<HwSensor[]>("list_sensors");
    sensors.value = Array.isArray(list) ? list : [];
  } catch {
    sensors.value = [];
    sensorsError.value = "读取失败，请稍后重试";
  } finally {
    sensorsLoading.value = false;
    sensorsRead.value = true;
  }
}

/* ============ SMART ============ */

/** disk_smart 新增字段。类型声明在视图内，避免改 tauri.ts。 */
interface DiskSmartRow extends DiskSmartInfo {
  temp_max_c?: number | null;
  read_errors?: number | null;
  write_errors?: number | null;
  start_stop?: number | null;
}

const smartDisks = ref<DiskSmartRow[]>([]);
const smartFailed = ref(false);
const smartLoading = ref(false);
const smartRead = ref(false);

async function readSmart() {
  smartLoading.value = true;
  const res = await diskSmart();
  smartLoading.value = false;
  smartRead.value = true;
  smartFailed.value = res === null;
  smartDisks.value = (res ?? []) as DiskSmartRow[];
}

function healthMeta(s?: string): { label: string; cls: string } {
  const v = (s ?? "").trim().toLowerCase();
  if (v === "unhealthy" || v.includes("fail") || v.includes("bad")) {
    return { label: "异常", cls: "text-[#e5484d]" };
  }
  if (v === "warning" || v.includes("caution")) {
    return { label: "警告", cls: "text-[#f5a623]" };
  }
  if (v === "healthy" || v === "good" || v === "ok") {
    return { label: "健康", cls: "text-[#30a46c]" };
  }
  return { label: s || "未知", cls: "text-[var(--muted)]" };
}

function smartHealthText(d: DiskSmartInfo): string | undefined {
  if (typeof d.health === "string" && d.health.trim()) return d.health;
  return d.status;
}

function diskTempClass(t: number | null | undefined) {
  if (t == null) return "";
  if (t > 60) return "text-[#e5484d]";
  if (t >= 45) return "text-[#f5a623]";
  return "";
}

const unlistens: UnlistenFn[] = [];

onMounted(async () => {
  void loadHw();
  void refreshTemps();
  void refreshUsage();
  startPolling();
  document.addEventListener("visibilitychange", onVisibilityChange);
  document.addEventListener("keydown", onGlobalKey);

  unlistens.push(
    await onCpuLoad((d) => {
      cpuPct.value = d.pct;
      if (typeof d.temp === "number") burnTemp.value = d.temp;
      if (typeof d.peak === "number") burnPeak.value = d.peak;
      if (d.pct >= 100) cpuRunning.value = false;
    }),
    await onCpuBurnEnd((d) => {
      cpuRunning.value = false;
      if (d.over_temp) {
        toast(
          `温度达到 ${d.limit}℃ 上限（峰值 ${d.peak}℃），压测已自动终止以保护硬件`,
          "error"
        );
      }
    }),
    await onGpuLoad((d) => {
      if (d.error) {
        gpuError.value = d.error;
        gpuRunning.value = false;
        return;
      }
      gpuPct.value = d.pct;
      gpuFps.value = d.fps;
      if (d.pct >= 100) gpuRunning.value = false;
    }),
    await onMemTestProgress((d) => {
      memPctTest.value = d.pct;
      memErrors.value = d.errors;
    }),
    await onMemTestDone((d) => {
      memRunning.value = false;
      memPctTest.value = 100;
      memResult.value = d;
    }),
  );
});

onUnmounted(() => {
  stopPolling();
  document.removeEventListener("visibilitychange", onVisibilityChange);
  document.removeEventListener("keydown", onGlobalKey);
  unlistens.forEach((u) => u());
  unlistens.length = 0;
});
</script>

<template>
  <div>
    <PageHeader title="硬件检测" desc="总览一屏看完。点开分类看完整规格，压测集中在最后一栏。">
      <template #extra>
        <Button
          variant="gray"
          size="sm"
          :title="hideVirtual ? '显示远程和虚拟显示器' : '隐藏远程和虚拟显示器'"
          @click="hideVirtual = !hideVirtual"
        >
          <EyeOff v-if="hideVirtual" class="h-3.5 w-3.5" />
          <Eye v-else class="h-3.5 w-3.5" />
          {{ hideVirtual ? `虚拟显示 ${hiddenGpuCount}` : "显示全部显卡" }}
        </Button>
        <Button variant="gray" size="sm" :disabled="!hw" title="复制纯文本硬件报告" @click="copyReport">
          <Copy class="h-3.5 w-3.5" /> 复制报告
        </Button>
        <Button variant="gray" size="icon" title="重新读取硬件信息" :disabled="hwLoading" @click="loadHw">
          <RefreshCw class="h-4 w-4" :class="hwLoading && 'animate-spin'" />
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

    <Spinner v-if="hwLoading && !hw" label="正在读取硬件信息…" />
    <EmptyState
      v-else-if="!hw"
      icon="🖥️"
      text="未能获取硬件信息"
      sub="请点击右上角刷新重试，或检查系统权限"
    />

    <template v-else>
      <!-- 总览：一屏 -->
      <div v-show="tab === 'overview'" class="space-y-4">
        <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
            <p class="text-[10px] text-[var(--muted)]">CPU 占用</p>
            <p class="mt-1 text-2xl font-bold tabular-nums text-[var(--text-strong)]">
              {{ cpuLive === null ? "…" : `${cpuLive.toFixed(0)}%` }}
            </p>
            <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-[var(--btn-gray)]">
              <div class="h-full rounded-full" :style="{ width: `${cpuLive ?? 0}%`, background: barColor(cpuLive ?? 0) }" />
            </div>
          </div>
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
            <p class="text-[10px] text-[var(--muted)]">内存占用</p>
            <p class="mt-1 truncate text-sm font-bold tabular-nums text-[var(--text-strong)]">
              {{ usage ? formatBytes(usage.mem_used) : "…" }}
              <span class="text-xs font-normal text-[var(--muted)]">/ {{ formatBytes(hw.memory_total) }}</span>
            </p>
            <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-[var(--btn-gray)]">
              <div class="h-full rounded-full" :style="{ width: `${memPct}%`, background: barColor(memPct) }" />
            </div>
          </div>
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3 text-center">
            <p class="text-[10px] text-[var(--muted)]">CPU 温度</p>
            <p class="mt-1 text-2xl font-bold tabular-nums" :class="tempClass(cpuT)">{{ tempText(cpuT) }}</p>
          </div>
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3 text-center">
            <p class="text-[10px] text-[var(--muted)]">GPU 温度</p>
            <p class="mt-1 text-2xl font-bold tabular-nums" :class="tempClass(gpuT)">{{ tempText(gpuT) }}</p>
          </div>
        </div>

        <Card class="p-5">
          <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
            <div class="flex min-w-0 flex-wrap items-center gap-2">
              <Flame class="h-4 w-4 shrink-0 text-[var(--primary)]" />
              <h4 class="font-semibold">温度传感器</h4>
            </div>
            <Button variant="gray" size="sm" :disabled="sensorsLoading" @click="readSensors">
              <RefreshCw class="h-3.5 w-3.5" :class="sensorsLoading && 'animate-spin'" />
              读取传感器
            </Button>
          </div>
          <Spinner v-if="sensorsLoading" label="正在读取温度传感器…" />
          <p v-else-if="sensorsError" class="text-sm text-[var(--muted)]">{{ sensorsError }}</p>
          <p v-else-if="!sensorsRead" class="text-sm text-[var(--muted)]">读取本机暴露给系统的温度传感器。</p>
          <EmptyState
            v-else-if="sensors.length === 0"
            icon="🌡️"
            text="未读到温度传感器"
            sub="本机可能未暴露传感器，这不是故障。不少主板和笔记本不会向系统提供这项数据。"
          />
          <div v-else class="space-y-2">
            <div
              v-for="(s, i) in sensors"
              :key="`${s.label}-${i}`"
              class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] px-3 py-2"
            >
              <span class="min-w-0 flex-1 truncate text-sm font-medium text-[var(--text-strong)]" :title="s.label || '未命名传感器'">
                {{ s.label || "未命名传感器" }}
              </span>
              <span class="shrink-0 text-sm font-semibold tabular-nums text-[var(--text-strong)]">{{ sensorTempText(s.temperature) }}</span>
              <span v-if="s.max != null" class="shrink-0 text-xs tabular-nums text-[var(--muted)]">最高 {{ sensorTempText(s.max) }}</span>
            </div>
          </div>
        </Card>

        <p v-if="osLine" class="truncate text-xs text-[var(--muted)]" :title="osLine">{{ osLine }}</p>

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <button type="button" class="rounded-xl border border-[var(--card-border)] bg-[var(--card)] p-4 text-left transition-colors hover:border-[var(--primary)]" @click="tab = 'cpu'">
            <p class="mb-1 flex items-center gap-1.5 text-xs text-[var(--muted)]"><Cpu class="h-3.5 w-3.5" /> 处理器</p>
            <p class="truncate text-sm font-semibold text-[var(--text-strong)]" :title="hw.cpu.name">{{ hw.cpu.name || "—" }}</p>
            <p class="mt-1 truncate text-xs text-[var(--muted)]">{{ cpuSub(hw.cpu) }}</p>
          </button>
          <button type="button" class="rounded-xl border border-[var(--card-border)] bg-[var(--card)] p-4 text-left transition-colors hover:border-[var(--primary)]" @click="tab = 'board'">
            <p class="mb-1 flex items-center gap-1.5 text-xs text-[var(--muted)]"><MemoryStick class="h-3.5 w-3.5" /> 内存 · {{ hw.memoryModules?.length || 0 }} 条</p>
            <p class="text-sm font-semibold text-[var(--text-strong)]">{{ formatBytes(hw.memory_total) }}</p>
            <p class="mt-1 truncate text-xs text-[var(--muted)]">{{ hw.board || "主板未报告" }}</p>
          </button>
          <button type="button" class="rounded-xl border border-[var(--card-border)] bg-[var(--card)] p-4 text-left transition-colors hover:border-[var(--primary)]" @click="tab = 'gpu'">
            <p class="mb-1 flex items-center gap-1.5 text-xs text-[var(--muted)]"><Monitor class="h-3.5 w-3.5" /> 显卡 · {{ visibleGpus.length }}</p>
            <p class="truncate text-sm font-semibold text-[var(--text-strong)]">{{ primaryGpu?.name || "—" }}</p>
            <p class="mt-1 truncate text-xs text-[var(--muted)]">{{ primaryGpu ? gpuSub(primaryGpu) : "未检测到显卡" }}</p>
          </button>
          <button type="button" class="rounded-xl border border-[var(--card-border)] bg-[var(--card)] p-4 text-left transition-colors hover:border-[var(--primary)]" @click="tab = 'disk'">
            <p class="mb-1 flex items-center gap-1.5 text-xs text-[var(--muted)]"><HardDrive class="h-3.5 w-3.5" /> 磁盘 · {{ hw.disks.length }}</p>
            <p class="truncate text-sm font-semibold text-[var(--text-strong)]">{{ hw.disks[0]?.model || "—" }}</p>
            <p class="mt-1 truncate text-xs text-[var(--muted)]">{{ hw.disks[0] ? diskSub(hw.disks[0]) : "未检测到磁盘" }}</p>
          </button>
        </div>
        <p class="text-[10px] text-[var(--muted)]">占用每 2 秒刷新，温度每 10 秒刷新。页面切走时暂停。点卡片进入对应规格。</p>
      </div>

      <!-- 处理器规格 -->
      <Card v-show="tab === 'cpu'" class="p-5">
        <div class="mb-3 flex items-center gap-2">
          <Cpu class="h-4 w-4 text-[var(--primary)]" />
          <h4 class="font-semibold">处理器规格</h4>
          <span class="text-xs text-[var(--muted)]">当前 {{ tempText(cpuT) }}</span>
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-2 sm:gap-x-8">
          <div v-for="[k, v] in cpuRows" :key="k" class="flex items-baseline justify-between gap-3 border-b border-[var(--card-border)] py-2.5">
            <span class="shrink-0 text-xs text-[var(--muted)]">{{ k }}</span>
            <span class="min-w-0 truncate text-right text-sm font-medium text-[var(--text-strong)]" :title="v">{{ v }}</span>
          </div>
        </div>
      </Card>

      <!-- 内存与主板 -->
      <div v-show="tab === 'board'" class="space-y-4">
        <Card class="p-5">
          <div class="mb-3 flex items-center gap-2">
            <CircuitBoard class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">主板与 BIOS</h4>
          </div>
          <div class="grid grid-cols-1 sm:grid-cols-2 sm:gap-x-8">
            <div v-for="[k, v] in boardRows" :key="k" class="flex items-baseline justify-between gap-3 border-b border-[var(--card-border)] py-2.5">
              <span class="shrink-0 text-xs text-[var(--muted)]">{{ k }}</span>
              <span class="min-w-0 truncate text-right text-sm font-medium text-[var(--text-strong)]" :title="v">{{ v }}</span>
            </div>
          </div>
        </Card>
        <Card class="p-5">
          <div class="mb-3 flex items-center gap-2">
            <MemoryStick class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">内存条</h4>
            <span class="text-xs text-[var(--muted)]">共 {{ formatBytes(hw.memory_total) }}</span>
          </div>
          <p v-if="!(hw.memoryModules || []).length" class="text-sm text-[var(--muted)]">未读到内存条明细</p>
          <div v-else class="space-y-3">
            <div v-for="(m, i) in hw.memoryModules" :key="i" class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
              <div class="flex flex-wrap items-baseline justify-between gap-2">
                <p class="text-sm font-semibold text-[var(--text-strong)]">{{ moduleBrief(m) || "内存条" }}</p>
                <p class="text-xs text-[var(--muted)]">{{ dash(m.slot) }}</p>
              </div>
              <p class="mt-1 text-xs text-[var(--muted)]">
                型号 {{ dash(m.part_number) }} · {{ dash(m.form_factor) }} · 插槽组 {{ dash(m.bank) }}
                · 运行 {{ m.configured_mhz ? `${m.configured_mhz} MHz` : "—" }}
                · {{ m.data_width ? `${m.data_width}-bit` : "位宽未报告" }}
              </p>
            </div>
          </div>
        </Card>
      </div>

      <!-- 显卡 -->
      <div v-show="tab === 'gpu'" class="space-y-3">
        <p v-if="!visibleGpus.length" class="text-sm text-[var(--muted)]">没有可显示的显卡。虚拟显示器已隐藏 {{ hiddenGpuCount }} 个。</p>
        <Card v-for="(g, i) in visibleGpus" :key="i" class="p-5">
          <div class="mb-3 flex flex-wrap items-center gap-2">
            <Monitor class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="min-w-0 flex-1 truncate font-semibold" :title="g.name">{{ g.name }}</h4>
            <span v-if="g.virtual" class="rounded-md bg-[var(--btn-gray)] px-1.5 py-0.5 text-[10px] text-[var(--muted)]">虚拟</span>
            <span class="text-xs" :class="tempClass(gpuT)">{{ tempText(gpuT) }}</span>
          </div>
          <div class="grid grid-cols-1 sm:grid-cols-2 sm:gap-x-8">
            <div v-for="[k, v] in [
              ['厂商', dash(g.vendor)],
              ['显存', vramText(g.vram) || (g.virtual ? '无' : '未报告')],
              ['驱动版本', dash(g.driver)],
              ['驱动日期', dash(g.driver_date)],
              ['当前分辨率', resText(g)],
              ['类型', g.virtual ? '远程 / 虚拟显示器' : '物理显卡'],
            ]" :key="k" class="flex items-baseline justify-between gap-3 border-b border-[var(--card-border)] py-2.5">
              <span class="shrink-0 text-xs text-[var(--muted)]">{{ k }}</span>
              <span class="min-w-0 truncate text-right text-sm font-medium text-[var(--text-strong)]" :title="v">{{ v }}</span>
            </div>
          </div>
        </Card>
      </div>

      <!-- 存储 -->
      <div v-show="tab === 'disk'" class="space-y-4">
        <Card v-for="(d, i) in hw.disks" :key="i" class="p-5">
          <div class="mb-2 flex flex-wrap items-center gap-2">
            <HardDrive class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="min-w-0 flex-1 truncate font-semibold" :title="d.model">{{ d.model || "未知磁盘" }}</h4>
            <span class="text-xs font-semibold" :class="healthMeta(d.health).cls">{{ healthMeta(d.health).label }}</span>
          </div>
          <p class="text-xs text-[var(--muted)]">
            {{ diskSub(d) }} · 序列号 {{ dash(d.serial) }} · 固件 {{ dash(d.firmware) }}
          </p>
        </Card>
        <Card v-if="hw.volumes?.length" class="p-5">
          <h4 class="mb-3 font-semibold">分区</h4>
          <div class="space-y-3">
            <div v-for="(v, i) in hw.volumes" :key="i">
              <div class="mb-1 flex items-baseline justify-between gap-3 text-xs">
                <span class="truncate font-medium text-[var(--text-strong)]">{{ v.mount }}</span>
                <span class="shrink-0 tabular-nums text-[var(--muted)]">
                  {{ formatBytes(v.total - v.free) }} / {{ formatBytes(v.total) }}
                </span>
              </div>
              <div class="h-1.5 overflow-hidden rounded-full bg-[var(--btn-gray)]">
                <div class="h-full rounded-full" :style="{ width: `${usedPct(v.total, v.free)}%`, background: barColor(usedPct(v.total, v.free)) }" />
              </div>
            </div>
          </div>
        </Card>

        <Card class="p-5">
          <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <HardDrive class="h-4 w-4 text-[var(--primary)]" />
              <h4 class="font-semibold">SMART</h4>
            </div>
            <Button variant="gray" size="sm" :disabled="smartLoading" @click="readSmart">
              <RefreshCw class="h-3.5 w-3.5" :class="smartLoading && 'animate-spin'" />
              {{ smartRead ? "重新读取" : "读取 SMART" }}
            </Button>
          </div>
          <Spinner v-if="smartLoading" label="正在读取磁盘 SMART 信息…" />
          <p v-else-if="smartFailed" class="text-sm text-[var(--muted)]">读取失败：需要管理员权限，或硬盘不支持 SMART</p>
          <p v-else-if="!smartRead" class="text-sm text-[var(--muted)]">读取健康状态、温度、通电时间、磨损、最高温、读写错误和启停次数</p>
          <p v-else-if="smartDisks.length === 0" class="text-sm text-[var(--muted)]">未发现磁盘</p>
          <div v-else class="overflow-x-auto rounded-xl border border-[var(--card-border)]">
            <table class="w-full min-w-[1080px] text-left text-xs">
              <thead>
                <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
                  <th class="px-3 py-2.5 font-medium">型号</th>
                  <th class="px-3 py-2.5 font-medium">序列号</th>
                  <th class="px-3 py-2.5 font-medium">总线</th>
                  <th class="px-3 py-2.5 font-medium">介质</th>
                  <th class="px-3 py-2.5 font-medium">健康</th>
                  <th class="px-3 py-2.5 text-right font-medium">温度</th>
                  <th class="px-3 py-2.5 text-right font-medium">最高温</th>
                  <th class="px-3 py-2.5 text-right font-medium">通电</th>
                  <th class="px-3 py-2.5 text-right font-medium">磨损</th>
                  <th class="px-3 py-2.5 text-right font-medium">读错误</th>
                  <th class="px-3 py-2.5 text-right font-medium">写错误</th>
                  <th class="px-3 py-2.5 text-right font-medium">启停次数</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-[var(--card-border)]">
                <tr v-for="(d, i) in smartDisks" :key="`${d.serial}-${i}`">
                  <td class="max-w-[180px] truncate px-3 py-2.5 font-medium" :title="d.model">{{ d.model || "—" }}</td>
                  <td class="max-w-[140px] truncate px-3 py-2.5 font-mono" :title="d.serial || ''">{{ d.serial || "—" }}</td>
                  <td class="px-3 py-2.5">{{ d.bus_type || "—" }}</td>
                  <td class="px-3 py-2.5">{{ d.media || "—" }}</td>
                  <td class="px-3 py-2.5 font-semibold" :class="healthMeta(smartHealthText(d)).cls">{{ healthMeta(smartHealthText(d)).label }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums" :class="diskTempClass(d.temperature_c)">{{ d.temperature_c == null ? "—" : d.temperature_c }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums" :class="diskTempClass(d.temp_max_c)">{{ dash(d.temp_max_c) }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums">{{ d.power_on_hours == null ? "—" : `${d.power_on_hours} h` }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums">{{ d.wear_pct == null ? "—" : `${d.wear_pct}%` }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums">{{ dash(d.read_errors) }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums">{{ dash(d.write_errors) }}</td>
                  <td class="px-3 py-2.5 text-right tabular-nums">{{ dash(d.start_stop) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </Card>
      </div>

      <!-- 压测：跟对应分类页共用同一块，避免两套状态 -->
      <div
        v-show="tab === 'cpu' || tab === 'board' || tab === 'gpu' || tab === 'disk' || tab === 'stress'"
        class="mt-4 grid grid-cols-1 gap-5"
        :class="tab === 'stress' && 'lg:grid-cols-2'"
      >
        <Card v-show="tab === 'cpu' || tab === 'stress'" class="p-5">
          <div class="mb-4 flex items-center gap-2">
            <Flame class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">CPU 烤机</h4>
          </div>
          <div class="mb-3 flex flex-wrap items-center gap-3">
            <Select v-model="cpuDur" :options="BURN_OPTS" class="w-36" />
            <div class="flex-1" />
            <Button variant="primary" size="sm" :disabled="cpuRunning" @click="cpuStart"><Play class="h-3.5 w-3.5" /> 开始</Button>
            <Button variant="danger" size="sm" :disabled="!cpuRunning" @click="cpuStop"><Square class="h-3.5 w-3.5" /> 停止</Button>
          </div>
          <Progress :value="cpuPct" :tone="cpuPct >= 100 ? 'success' : 'warning'" />
          <div class="mt-2 flex items-center justify-between gap-2 text-xs text-[var(--muted)]">
            <span>{{ cpuRunning ? "烤机进行中，请留意温度" : "多线程满载，到时自动停" }}</span>
            <span v-if="cpuRunning" class="font-mono font-semibold tabular-nums text-[var(--text)]">剩余 {{ fmtCountdown(cpuRemain) }}</span>
          </div>
          <!-- 超温保护状态 -->
          <div v-if="cpuRunning || burnPeak > 0" class="mt-2 flex flex-wrap items-center gap-2 text-xs">
            <span :class="burnTemp !== null && burnTemp >= 85 ? 'text-[#e5484d]' : 'text-[var(--muted)]'">
              当前 {{ burnTemp === null ? "—" : `${burnTemp}℃` }}
            </span>
            <span class="text-[var(--muted)]">·</span>
            <span class="text-[var(--muted)]">峰值 {{ burnPeak > 0 ? `${burnPeak}℃` : "—" }}</span>
            <span class="rounded-md bg-[#f5a623]/15 px-1.5 py-0.5 text-[10px] text-[#f5a623]">
              超过设定上限将自动终止
            </span>
          </div>
        </Card>

        <Card v-show="tab === 'gpu' || tab === 'stress'" class="p-5">
          <div class="mb-4 flex items-center gap-2">
            <Zap class="h-4 w-4 text-[#a78bfa]" />
            <h4 class="font-semibold">GPU 烤机</h4>
          </div>
          <div class="mb-3 flex flex-wrap items-center gap-3">
            <Select v-model="gpuDur" :options="BURN_OPTS" class="w-36" />
            <div class="flex-1" />
            <Button variant="primary" size="sm" :disabled="gpuRunning || !!gpuError" @click="gpuStart"><Play class="h-3.5 w-3.5" /> 开始</Button>
            <Button variant="danger" size="sm" :disabled="!gpuRunning" @click="gpuStop"><Square class="h-3.5 w-3.5" /> 停止</Button>
          </div>
          <div class="h-2 w-full overflow-hidden rounded-full bg-[var(--btn-gray)]">
            <div class="h-full rounded-full transition-[width] duration-200" :style="{ width: `${Math.min(100, Math.max(0, gpuPct))}%`, background: 'linear-gradient(90deg, #7c3aed, #a78bfa)' }" />
          </div>
          <p v-if="gpuError" class="mt-2 text-xs text-[#e5484d]">GPU 烤机出错：{{ gpuError }}</p>
          <div v-else class="mt-2 flex items-center justify-between gap-2 text-xs text-[var(--muted)]">
            <span>{{ gpuRunning ? "GPU 压测进行中" : "计算着色器满载，到时自动停" }}</span>
            <span v-if="gpuRunning" class="font-mono font-semibold tabular-nums text-[var(--text)]">FPS {{ gpuFps }} · {{ fmtCountdown(gpuRemain) }}</span>
          </div>
        </Card>

        <Card v-show="tab === 'board' || tab === 'stress'" class="p-5">
          <div class="mb-4 flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <MemoryStick class="h-4 w-4 text-[var(--primary)]" />
              <h4 class="font-semibold">内存测试</h4>
            </div>
            <Button variant="primary" size="sm" :disabled="memRunning" @click="memStart">
              <Loader2 v-if="memRunning" class="h-3.5 w-3.5 animate-spin" />
              <Play v-else class="h-3.5 w-3.5" />
              {{ memRunning ? "测试中" : "开始" }}
            </Button>
          </div>
          <Progress :value="memPctTest" :tone="memResult ? (memResult.errors === 0 ? 'success' : 'danger') : 'primary'" />
          <div
            v-if="memResult"
            class="mt-3 rounded-xl border p-3 text-center"
            :class="memResult.errors === 0 ? 'border-[#30a46c]/40 bg-[#30a46c]/10' : 'border-[#e5484d]/40 bg-[#e5484d]/10'"
          >
            <p class="font-bold" :class="memResult.errors === 0 ? 'text-[#30a46c]' : 'text-[#e5484d]'">
              {{ memResult.errors === 0 ? "无错误" : `发现 ${memResult.errors} 个错误` }}
            </p>
            <p class="mt-1 text-xs text-[var(--muted)]">{{ memResult.blocks_total }} 块 · {{ memResult.seconds }} 秒</p>
          </div>
          <p v-else class="mt-2 text-xs text-[var(--muted)]">
            {{ memRunning ? `进度 ${Math.round(memPctTest)}%${memErrors ? `，错误 ${memErrors}` : ""}` : "写入固定图案和随机图案后读回校验" }}
          </p>
        </Card>

        <Card v-show="tab === 'disk' || tab === 'stress'" class="p-5">
          <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <HardDrive class="h-4 w-4 text-[var(--primary)]" />
              <h4 class="font-semibold">磁盘测速</h4>
            </div>
            <div class="flex items-center gap-2">
              <Select v-model="diskSel" :options="driveOpts" class="w-28" />
              <Button variant="primary" size="sm" :disabled="diskTesting" @click="runDiskSpeed">
                <Loader2 v-if="diskTesting" class="h-3.5 w-3.5 animate-spin" />
                <Play v-else class="h-3.5 w-3.5" />
                {{ diskTesting ? "测速中" : "开始" }}
              </Button>
            </div>
          </div>
          <div class="grid grid-cols-2 gap-3">
            <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3 text-center">
              <p class="text-[10px] text-[var(--muted)]">读取</p>
              <p class="text-2xl font-bold tabular-nums">{{ fmtMbps(diskRes?.read_mbps) }}<span class="ml-1 text-xs font-normal text-[var(--muted)]">MB/s</span></p>
            </div>
            <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3 text-center">
              <p class="text-[10px] text-[var(--muted)]">写入</p>
              <p class="text-2xl font-bold tabular-nums">{{ fmtMbps(diskRes?.write_mbps) }}<span class="ml-1 text-xs font-normal text-[var(--muted)]">MB/s</span></p>
            </div>
          </div>
          <p class="mt-2 text-[10px] text-[var(--muted)]">顺序读写 512MB，会占用目标盘空间，测完删除临时文件</p>
        </Card>

        <Card v-show="tab === 'gpu' || tab === 'stress'" class="p-5">
          <div class="mb-3 flex items-center gap-2">
            <Grid3x3 class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">坏点检测</h4>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <Button v-for="c in PIXEL_COLORS" :key="c.value" variant="gray" size="sm" @click="pixelColor = c.value">
              <span class="mr-1.5 inline-block h-3 w-3 rounded-full border border-[var(--card-border)]" :style="{ background: c.value }" />
              {{ c.label }}
            </Button>
          </div>
          <p class="mt-2 text-[10px] text-[var(--muted)]">全屏纯色，点击或按 Esc 退出</p>
        </Card>
      </div>
    </template>

    <div
      v-if="pixelColor"
      class="fixed inset-0 z-[9999] cursor-pointer"
      :style="{ background: pixelColor }"
      title="点击退出（Esc 也可退出）"
      @click="closePixel"
    />
  </div>
</template>
