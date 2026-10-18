<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  CircuitBoard, Cpu, Fan, Flame, Grid3x3, HardDrive, Loader2, MemoryStick, Monitor,
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
  gpuTemp, hwInfo, memTestStart, onCpuLoad, onGpuLoad, onMemTestDone, onMemTestProgress,
  type DiskSmartInfo, type DiskSpeed, type HwInfo, type MemTestResult,
} from "../lib/tauri";
import { formatBytes } from "../lib/utils";

/* ============ 通用：烤机时长选项 ============ */

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

/* ============ 硬件信息 ============ */

const hw = ref<HwInfo | null>(null);
const hwLoading = ref(false);

const memLine = computed(() => {
  const mods = hw.value?.memoryModules ?? [];
  if (mods.length === 0) return "";
  return mods
    .map((m) => {
      const who = (m.manufacturer || "").trim();
      const cap = m.capacity_gb ? `${m.capacity_gb}GB` : "";
      const spd = m.speed ? `${m.speed}MHz` : "";
      return [who, cap, spd].filter(Boolean).join(" ");
    })
    .filter(Boolean)
    .join(" · ");
});

async function loadHw() {
  hwLoading.value = true;
  hw.value = await hwInfo();
  hwLoading.value = false;
}

function vramText(vram: number) {
  if (!vram || vram <= 0) return "";
  // 后端已换算为 MB；若仍是字节（旧数据 > 64GB 当 MB 不可能）则再除一次
  const mb = vram > 65536 ? Math.round(vram / 1048576) : Math.round(vram);
  return mb > 0 ? `${mb} MB 显存` : "";
}

/* ============ CPU 烤机 ============ */

const cpuDur = ref("60");
const cpuRunning = ref(false);
const cpuPct = ref(0);
const cpuRemain = computed(() =>
  Math.max(0, Math.ceil((Number(cpuDur.value) * (100 - cpuPct.value)) / 100))
);

async function cpuStart() {
  if (cpuRunning.value) return;
  cpuPct.value = 0;
  cpuRunning.value = true;
  await cpuBurnStart(Number(cpuDur.value));
}

async function cpuStop() {
  await cpuBurnStop();
  cpuRunning.value = false;
  cpuPct.value = 0;
}

/* ============ GPU 烤机（紫色风格） ============ */

const gpuDur = ref("60");
const gpuRunning = ref(false);
const gpuPct = ref(0);
const gpuFps = ref(0);
const gpuError = ref<string | null>(null);
const gpuRemain = computed(() =>
  Math.max(0, Math.ceil((Number(gpuDur.value) * (100 - gpuPct.value)) / 100))
);

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
const memPct = ref(0);
const memErrors = ref(0);
const memResult = ref<MemTestResult | null>(null);

async function memStart() {
  if (memRunning.value) return;
  memResult.value = null;
  memPct.value = 0;
  memErrors.value = 0;
  memRunning.value = true;
  await memTestStart();
}

/* ============ 磁盘测速 ============ */

const DRIVE_OPTS = ["C", "D", "E", "F", "G", "H"].map((d) => ({
  value: `${d}:\\`,
  label: `${d} 盘`,
}));
const diskSel = ref("C:\\");
const diskTesting = ref(false);
const diskRes = ref<DiskSpeed | null>(null);

async function runDiskSpeed() {
  if (diskTesting.value) return;
  diskTesting.value = true;
  diskRes.value = null;
  diskRes.value = await diskSpeed(diskSel.value);
  diskTesting.value = false;
}

/** 测速结果可能缺字段（旧契约曾把 read_mbps 序列化成 readMbps），不能直接 toFixed */
function fmtMbps(n: number | null | undefined) {
  return typeof n === "number" && Number.isFinite(n) ? n.toFixed(1) : "—";
}

/* ============ 坏点检测（纯前端全屏纯色覆盖层） ============ */

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

/* ============ 温度监控（10s 轮询；页面切走/最小化时暂停） ============ */

const cpuT = ref<number | null>(null);
const gpuT = ref<number | null>(null);
const tempLoading = ref(false);

async function refreshTemps() {
  tempLoading.value = true;
  const [c, g] = await Promise.all([cpuTemp(), gpuTemp()]);
  // 后端温度传感器不可用时返回 null，UI 显示 "—"
  cpuT.value = c;
  gpuT.value = g;
  tempLoading.value = false;
}

let tempTimer: number | undefined;

function startPolling() {
  if (tempTimer !== undefined) return;
  tempTimer = window.setInterval(() => {
    // 页面在后台（最小化到托盘）时跳过本次采集
    if (document.visibilityState === "visible") void refreshTemps();
  }, 10_000);
}

function stopPolling() {
  if (tempTimer !== undefined) {
    window.clearInterval(tempTimer);
    tempTimer = undefined;
  }
}

function onVisibilityChange() {
  // 重新可见时立即刷新一次
  if (document.visibilityState === "visible") void refreshTemps();
}

/** 温度配色：≥80 红 / ≥60 黄 / 其余绿，null 灰 */
function tempClass(t: number | null) {
  if (t === null) return "text-[var(--muted)]";
  if (t >= 80) return "text-[#e5484d]";
  if (t >= 60) return "text-[#f5a623]";
  return "text-[#30a46c]";
}
function tempText(t: number | null) {
  return t === null ? "—" : `${t}℃`;
}

/* ============ 磁盘健康（SMART） ============ */

const smartDisks = ref<DiskSmartInfo[]>([]);
const smartFailed = ref(false);
const smartLoading = ref(false);
const smartRead = ref(false);

async function readSmart() {
  smartLoading.value = true;
  const res = await diskSmart();
  smartLoading.value = false;
  smartRead.value = true;
  smartFailed.value = res === null;
  smartDisks.value = res ?? [];
}

/** 健康状态映射：Healthy 绿 / Warning 黄 / Unhealthy 红，未知状态原样灰色显示 */
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

/** 磁盘温度配色：>60 红 / 45~60 黄 */
/** 优先用 HealthStatus（Healthy），没有再用 OperationalStatus（OK） */
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

/* ============ 生命周期：加载数据 + 注册事件（onUnmounted 全部清理） ============ */

const unlistens: UnlistenFn[] = [];

onMounted(async () => {
  void loadHw();
  void refreshTemps();
  startPolling();
  document.addEventListener("visibilitychange", onVisibilityChange);
  document.addEventListener("keydown", onGlobalKey);

  unlistens.push(
    await onCpuLoad((d) => {
      cpuPct.value = d.pct;
      if (d.pct >= 100) cpuRunning.value = false;
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
      memPct.value = d.pct;
      memErrors.value = d.errors;
    }),
    await onMemTestDone((d) => {
      memRunning.value = false;
      memPct.value = 100;
      memResult.value = d;
    })
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
    <PageHeader title="硬件检测" desc="硬件信息、烤机压测、磁盘测速、内存测试与屏幕坏点检测" />

    <!-- ===== 概览 ===== -->
    <p class="mb-3 text-xs font-semibold tracking-wider text-[var(--muted)]">概览</p>
    <div class="grid grid-cols-1 gap-5 lg:grid-cols-3">
      <!-- 硬件信息 -->
      <Card class="p-5 lg:col-span-2">
        <div class="mb-4 flex items-center justify-between">
          <div class="flex items-center gap-2">
            <CircuitBoard class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">硬件信息</h4>
          </div>
          <Button variant="gray" size="icon" title="重新读取硬件信息" :disabled="hwLoading" @click="loadHw">
            <RefreshCw class="h-4 w-4" :class="hwLoading && 'animate-spin'" />
          </Button>
        </div>

        <Spinner v-if="hwLoading" label="正在读取硬件信息…" />
        <EmptyState
          v-else-if="!hw"
          icon="🖥️"
          text="未能获取硬件信息"
          sub="请点击右上角刷新重试，或检查系统权限"
        />

        <div v-else class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4">
            <p class="mb-1.5 flex items-center gap-1.5 text-xs text-[var(--muted)]">
              <Cpu class="h-3.5 w-3.5" /> 处理器
            </p>
            <p class="truncate text-sm font-semibold text-[var(--text-strong)]" :title="hw.cpu.name">
              {{ hw.cpu.name || "—" }}
            </p>
            <p class="mt-1 text-xs text-[var(--muted)]">{{ hw.cpu.cores }} 核 / {{ hw.cpu.threads }} 线程</p>
          </div>

          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4">
            <p class="mb-1.5 flex items-center gap-1.5 text-xs text-[var(--muted)]">
              <MemoryStick class="h-3.5 w-3.5" /> 内存总量
            </p>
            <p class="text-sm font-semibold text-[var(--text-strong)]">{{ formatBytes(hw.memory_total) }}</p>
            <p v-if="memLine" class="mt-1 truncate text-xs text-[var(--muted)]" :title="memLine">{{ memLine }}</p>
          </div>

          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4">
            <p class="mb-1.5 flex items-center gap-1.5 text-xs text-[var(--muted)]">
              <CircuitBoard class="h-3.5 w-3.5" /> 主板
            </p>
            <p class="truncate text-sm font-semibold text-[var(--text-strong)]" :title="hw.board">
              {{ hw.board || "—" }}
            </p>
          </div>

          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4">
            <p class="mb-1.5 flex items-center gap-1.5 text-xs text-[var(--muted)]">
              <Monitor class="h-3.5 w-3.5" /> 显卡（{{ hw.gpus.length }}）
            </p>
            <p v-for="(g, i) in hw.gpus" :key="i" class="truncate text-sm font-semibold text-[var(--text-strong)]" :title="g.name">
              {{ g.name || "—" }}
              <span v-if="vramText(g.vram)" class="ml-1 text-xs font-normal text-[var(--muted)]">{{ vramText(g.vram) }}</span>
            </p>
            <p v-if="hw.gpus.length === 0" class="text-sm text-[var(--muted)]">—</p>
          </div>

          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4 sm:col-span-2">
            <p class="mb-1.5 flex items-center gap-1.5 text-xs text-[var(--muted)]">
              <HardDrive class="h-3.5 w-3.5" /> 磁盘（{{ hw.disks.length }}）
            </p>
            <p v-for="(d, i) in hw.disks" :key="i" class="truncate text-sm text-[var(--text)]" :title="d.model">
              {{ d.model || "—" }}
              <span class="ml-1 text-xs text-[var(--muted)]">{{ formatBytes(d.size) }}</span>
            </p>
            <p v-if="hw.disks.length === 0" class="text-sm text-[var(--muted)]">—</p>
          </div>
        </div>
      </Card>

      <!-- 温度监控 -->
      <Card class="p-5">
        <div class="mb-4 flex items-center justify-between">
          <div class="flex items-center gap-2">
            <Fan class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">温度监控</h4>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-[10px] text-[var(--muted)]">10s 自动刷新</span>
            <Button variant="gray" size="icon" title="手动刷新温度" :disabled="tempLoading" @click="refreshTemps">
              <RefreshCw class="h-4 w-4" :class="tempLoading && 'animate-spin'" />
            </Button>
          </div>
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4 text-center">
            <p class="mb-1 text-xs text-[var(--muted)]">CPU 温度</p>
            <p class="text-3xl font-bold tabular-nums" :class="tempClass(cpuT)">{{ tempText(cpuT) }}</p>
          </div>
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4 text-center">
            <p class="mb-1 text-xs text-[var(--muted)]">GPU 温度</p>
            <p class="text-3xl font-bold tabular-nums" :class="tempClass(gpuT)">{{ tempText(gpuT) }}</p>
          </div>
        </div>
        <p class="mt-3 text-[10px] text-[var(--muted)]">
          传感器不可用或驱动不支持时显示「—」；页面切走或最小化时自动暂停刷新
        </p>
      </Card>
    </div>

    <!-- ===== 烤机与测试 ===== -->
    <p class="mb-3 mt-6 text-xs font-semibold tracking-wider text-[var(--muted)]">烤机与测试</p>
    <div class="grid grid-cols-1 gap-5 lg:grid-cols-2">
      <!-- CPU 烤机 -->
      <Card class="p-5">
        <div class="mb-4 flex items-center gap-2">
          <Flame class="h-4 w-4 text-[var(--primary)]" />
          <h4 class="font-semibold">CPU 烤机</h4>
        </div>
        <div class="mb-3 flex items-center gap-3">
          <Select v-model="cpuDur" :options="BURN_OPTS" class="w-36" />
          <div class="flex-1" />
          <Button variant="primary" size="sm" :disabled="cpuRunning" @click="cpuStart">
            <Play class="h-3.5 w-3.5" /> 开始
          </Button>
          <Button variant="danger" size="sm" :disabled="!cpuRunning" @click="cpuStop">
            <Square class="h-3.5 w-3.5" /> 停止
          </Button>
        </div>
        <Progress :value="cpuPct" :tone="cpuPct >= 100 ? 'success' : 'warning'" />
        <div class="mt-2 flex items-center justify-between text-xs text-[var(--muted)]">
          <span>{{ cpuRunning ? "烤机进行中，请留意温度" : "选择时长后开始 CPU 稳定性压测" }}</span>
          <span v-if="cpuRunning" class="font-mono font-semibold tabular-nums text-[var(--text)]">
            剩余 {{ fmtCountdown(cpuRemain) }}（{{ Math.round(cpuPct) }}%）
          </span>
        </div>
      </Card>

      <!-- GPU 烤机（紫色风格） -->
      <Card class="p-5">
        <div class="mb-4 flex items-center gap-2">
          <Zap class="h-4 w-4 text-[#a78bfa]" />
          <h4 class="font-semibold">GPU 烤机</h4>
          <span class="rounded-md bg-[#8b5cf6]/15 px-1.5 py-0.5 text-[10px] font-medium text-[#a78bfa]">GPU</span>
        </div>
        <div class="mb-3 flex items-center gap-3">
          <Select v-model="gpuDur" :options="BURN_OPTS" class="w-36" />
          <div class="flex-1" />
          <Button variant="primary" size="sm" :disabled="gpuRunning || !!gpuError" @click="gpuStart">
            <Play class="h-3.5 w-3.5" /> 开始
          </Button>
          <Button variant="danger" size="sm" :disabled="!gpuRunning" @click="gpuStop">
            <Square class="h-3.5 w-3.5" /> 停止
          </Button>
        </div>

        <!-- 紫色进度条 -->
        <div class="h-2 w-full overflow-hidden rounded-full bg-[var(--btn-gray)]">
          <div
            class="h-full rounded-full transition-[width] duration-200"
            :style="{ width: `${Math.min(100, Math.max(0, gpuPct))}%`, background: 'linear-gradient(90deg, #7c3aed, #a78bfa)' }"
          />
        </div>

        <div class="mt-2 flex items-center justify-between text-xs">
          <template v-if="gpuError">
            <span class="text-[#e5484d]">GPU 烤机出错：{{ gpuError }}（已禁用，请检查显卡驱动）</span>
          </template>
          <template v-else>
            <span class="text-[var(--muted)]">{{ gpuRunning ? "GPU 压测进行中" : "选择时长后开始 GPU 稳定性压测" }}</span>
            <span v-if="gpuRunning" class="font-mono font-semibold tabular-nums text-[var(--text)]">
              FPS {{ gpuFps }} · 剩余 {{ fmtCountdown(gpuRemain) }}（{{ Math.round(gpuPct) }}%）
            </span>
          </template>
        </div>
      </Card>

      <!-- 内存测试 -->
      <Card class="p-5">
        <div class="mb-4 flex items-center justify-between">
          <div class="flex items-center gap-2">
            <MemoryStick class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">内存测试</h4>
          </div>
          <Button variant="primary" size="sm" :disabled="memRunning" @click="memStart">
            <Loader2 v-if="memRunning" class="h-3.5 w-3.5 animate-spin" />
            <Play v-else class="h-3.5 w-3.5" />
            {{ memRunning ? "测试中…" : "开始测试" }}
          </Button>
        </div>

        <Progress :value="memPct" :tone="memResult ? (memResult.errors === 0 ? 'success' : 'danger') : 'primary'" />
        <p v-if="memRunning" class="mt-2 text-xs text-[var(--muted)]">
          正在检测内存块… 进度 {{ Math.round(memPct) }}%<template v-if="memErrors > 0">，已发现 <span class="font-semibold text-[#e5484d]">{{ memErrors }} 个错误</span></template>
        </p>

        <!-- 结果面板 -->
        <div
          v-if="memResult"
          class="mt-3 rounded-xl border p-4 text-center"
          :class="memResult.errors === 0 ? 'border-[#30a46c]/40 bg-[#30a46c]/10' : 'border-[#e5484d]/40 bg-[#e5484d]/10'"
        >
          <p class="text-lg font-bold" :class="memResult.errors === 0 ? 'text-[#30a46c]' : 'text-[#e5484d]'">
            {{ memResult.errors === 0 ? "✓ 无错误" : `✗ 发现 ${memResult.errors} 个错误` }}
          </p>
          <p class="mt-1 text-xs text-[var(--muted)]">
            共检测 {{ memResult.blocks_total }} 个内存块 · 耗时 {{ memResult.seconds }} 秒
          </p>
        </div>
        <p v-else-if="!memRunning" class="mt-2 text-xs text-[var(--muted)]">
          对物理内存进行读写校验，检测内存条稳定性
        </p>
      </Card>

      <!-- 磁盘测速 -->
      <Card class="p-5">
        <div class="mb-4 flex items-center justify-between">
          <div class="flex items-center gap-2">
            <HardDrive class="h-4 w-4 text-[var(--primary)]" />
            <h4 class="font-semibold">磁盘测速</h4>
          </div>
          <div class="flex items-center gap-3">
            <Select v-model="diskSel" :options="DRIVE_OPTS" class="w-32" />
            <Button variant="primary" size="sm" :disabled="diskTesting" @click="runDiskSpeed">
              <Loader2 v-if="diskTesting" class="h-3.5 w-3.5 animate-spin" />
              <Play v-else class="h-3.5 w-3.5" />
              {{ diskTesting ? "测速中…" : "开始测速" }}
            </Button>
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4 text-center">
            <p class="mb-1 text-xs text-[var(--muted)]">读取速度</p>
            <p class="text-3xl font-bold tabular-nums text-[var(--text-strong)]">
              {{ fmtMbps(diskRes?.read_mbps) }}
              <span class="ml-1 text-sm font-normal text-[var(--muted)]">MB/s</span>
            </p>
          </div>
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4 text-center">
            <p class="mb-1 text-xs text-[var(--muted)]">写入速度</p>
            <p class="text-3xl font-bold tabular-nums text-[var(--text-strong)]">
              {{ fmtMbps(diskRes?.write_mbps) }}
              <span class="ml-1 text-sm font-normal text-[var(--muted)]">MB/s</span>
            </p>
          </div>
        </div>
        <p v-if="!diskRes && !diskTesting" class="mt-2 text-xs text-[var(--muted)]">
          选择目标磁盘分区后开始顺序读写测速
        </p>
      </Card>
    </div>

    <!-- ===== 磁盘健康 ===== -->
    <p class="mb-3 mt-6 text-xs font-semibold tracking-wider text-[var(--muted)]">磁盘健康</p>
    <Card class="p-5">
      <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <HardDrive class="h-4 w-4 text-[var(--primary)]" />
          <h4 class="font-semibold">磁盘健康（SMART）</h4>
          <span v-if="smartRead && !smartFailed" class="text-xs font-normal text-[var(--muted)]">
            {{ smartDisks.length }} 块磁盘
          </span>
        </div>
        <Button variant="gray" size="sm" :disabled="smartLoading" @click="readSmart">
          <RefreshCw class="h-3.5 w-3.5" :class="smartLoading && 'animate-spin'" />
          {{ smartRead ? "重新读取 SMART" : "读取 SMART" }}
        </Button>
      </div>

      <!-- 读取中 -->
      <Spinner v-if="smartLoading" label="正在读取磁盘 SMART 信息…" />

      <!-- 读取失败：灰色提示 -->
      <div
        v-else-if="smartFailed"
        class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] px-4 py-6 text-center text-sm text-[var(--muted)]"
      >
        读取失败：需要管理员权限或硬盘不支持 SMART
      </div>

      <!-- 未读取 -->
      <div v-else-if="!smartRead" class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] px-4 py-6 text-center text-sm text-[var(--muted)]">
        点击「读取 SMART」获取磁盘健康状态、温度与磨损信息
      </div>

      <!-- 空列表 -->
      <div v-else-if="smartDisks.length === 0" class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] px-4 py-6 text-center text-sm text-[var(--muted)]">
        未发现磁盘
      </div>

      <!-- SMART 表格 -->
      <div v-else class="overflow-x-auto rounded-xl border border-[var(--card-border)]">
        <table class="w-full min-w-[720px] text-left text-xs">
          <thead>
            <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
              <th class="px-3 py-2.5 font-medium">型号</th>
              <th class="px-3 py-2.5 font-medium">序列号</th>
              <th class="px-3 py-2.5 font-medium">总线</th>
              <th class="px-3 py-2.5 font-medium">介质</th>
              <th class="px-3 py-2.5 font-medium">健康状态</th>
              <th class="px-3 py-2.5 text-right font-medium">温度℃</th>
              <th class="px-3 py-2.5 text-right font-medium">通电小时</th>
              <th class="px-3 py-2.5 text-right font-medium">损耗%</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-[var(--card-border)]">
            <tr v-for="(d, i) in smartDisks" :key="`${d.serial}-${i}`" class="transition-colors hover:bg-[var(--card-soft)]">
              <td class="max-w-[180px] truncate px-3 py-2.5 font-medium text-[var(--text-strong)]" :title="d.model">
                {{ d.model || "—" }}
              </td>
              <td class="px-3 py-2.5 font-mono">{{ d.serial || "—" }}</td>
              <td class="px-3 py-2.5">{{ d.bus_type || "—" }}</td>
              <td class="px-3 py-2.5">{{ d.media || "—" }}</td>
              <td class="px-3 py-2.5 font-semibold" :class="healthMeta(smartHealthText(d)).cls">
                {{ healthMeta(smartHealthText(d)).label }}
              </td>
              <td class="px-3 py-2.5 text-right tabular-nums" :class="diskTempClass(d.temperature_c)">
                {{ d.temperature_c == null ? "—" : `${d.temperature_c}` }}
              </td>
              <td class="px-3 py-2.5 text-right tabular-nums">
                {{ d.power_on_hours == null ? "—" : `${d.power_on_hours} h` }}
              </td>
              <td class="px-3 py-2.5 text-right tabular-nums">
                {{ d.wear_pct == null ? "—" : `${d.wear_pct}%` }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </Card>

    <!-- ===== 屏幕测试 ===== -->
    <p class="mb-3 mt-6 text-xs font-semibold tracking-wider text-[var(--muted)]">屏幕测试</p>
    <Card class="p-5">
      <div class="mb-3 flex items-center gap-2">
        <Grid3x3 class="h-4 w-4 text-[var(--primary)]" />
        <h4 class="font-semibold">坏点检测</h4>
      </div>
      <div class="flex flex-wrap items-center gap-3">
        <Button
          v-for="c in PIXEL_COLORS"
          :key="c.value"
          variant="gray"
          size="sm"
          class="!px-4"
          @click="pixelColor = c.value"
        >
          <span class="mr-1.5 inline-block h-3 w-3 rounded-full border border-[var(--card-border)]" :style="{ background: c.value }" />
          {{ c.label }}
        </Button>
        <span class="text-xs text-[var(--muted)]">点击颜色进入全屏纯色模式，按 Esc 或点击屏幕退出</span>
      </div>
    </Card>

    <!-- 全屏坏点检测覆盖层 -->
    <div
      v-if="pixelColor"
      class="fixed inset-0 z-[9999] cursor-pointer"
      :style="{ background: pixelColor }"
      title="点击退出（Esc 也可退出）"
      @click="closePixel"
    />
  </div>
</template>
