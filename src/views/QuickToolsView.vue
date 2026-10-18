<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  AlarmClock, Loader2, Power, RefreshCw, Search, Square, Wrench, X,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import {
  cancelShutdown, listPorts, openSystemTool, scheduleShutdown, type PortRow,
} from "../lib/tauri";
import { toast } from "../lib/toast";

type Tab = "ports" | "tools" | "power";
const TABS: { id: Tab; label: string }[] = [
  { id: "ports", label: "端口占用" },
  { id: "tools", label: "系统入口" },
  { id: "power", label: "定时关机" },
];
const tab = ref<Tab>("ports");

/* ============ 端口 ============ */

const ports = ref<PortRow[]>([]);
const portLoading = ref(false);
const portFilter = ref("");
const portProto = ref("all");

const PROTO_OPTS = [
  { value: "all", label: "全部协议" },
  { value: "TCP", label: "仅 TCP" },
  { value: "UDP", label: "仅 UDP" },
];

const shownPorts = computed(() => {
  const kw = portFilter.value.trim().toLowerCase();
  return ports.value.filter((p) => {
    if (portProto.value !== "all" && p.proto !== portProto.value) return false;
    if (!kw) return true;
    return (
      String(p.port).includes(kw) ||
      String(p.pid) === kw ||
      p.name.toLowerCase().includes(kw) ||
      p.addr.includes(kw)
    );
  });
});

async function loadPorts() {
  portLoading.value = true;
  ports.value = (await listPorts()) ?? [];
  portLoading.value = false;
}

/* ============ 系统入口 ============ */

const TOOL_GROUPS: { title: string; items: { id: string; label: string }[] }[] = [
  {
    title: "系统管理",
    items: [
      { id: "control", label: "控制面板" },
      { id: "appwiz", label: "程序和功能" },
      { id: "sysprops", label: "系统属性" },
      { id: "msinfo", label: "系统信息" },
      { id: "taskmgr", label: "任务管理器" },
      { id: "resmon", label: "资源监视器" },
    ],
  },
  {
    title: "硬件与设备",
    items: [
      { id: "devmgmt", label: "设备管理器" },
      { id: "diskmgmt", label: "磁盘管理" },
      { id: "display", label: "显示设置" },
      { id: "sound", label: "声音" },
      { id: "powercfg", label: "电源选项" },
      { id: "fonts", label: "字体设置" },
    ],
  },
  {
    title: "服务与网络",
    items: [
      { id: "services", label: "服务" },
      { id: "network", label: "网络连接" },
      { id: "firewall", label: "防火墙" },
      { id: "eventvwr", label: "事件查看器" },
      { id: "regedit", label: "注册表编辑器" },
      { id: "userpass", label: "用户账户" },
    ],
  },
  {
    title: "常用文件夹",
    items: [
      { id: "temp", label: "临时文件夹" },
      { id: "startupfolder", label: "启动文件夹" },
      { id: "hostsfolder", label: "Hosts 目录" },
      { id: "env", label: "环境变量面板" },
      { id: "timedate", label: "日期和时间" },
    ],
  },
];

const openingTool = ref<string | null>(null);

async function openTool(id: string) {
  if (openingTool.value) return;
  openingTool.value = id;
  try {
    const label = await openSystemTool(id);
    toast(`已打开 ${label}`, "success");
  } catch (err) {
    toast(String(err), "error");
  } finally {
    openingTool.value = null;
  }
}

/* ============ 定时关机 ============ */

const POWER_OPTS = [
  { value: "shutdown", label: "关机" },
  { value: "restart", label: "重启" },
  { value: "logoff", label: "注销" },
];
const PRESET_OPTS = [
  { value: "0", label: "立即" },
  { value: "60", label: "1 分钟后" },
  { value: "300", label: "5 分钟后" },
  { value: "600", label: "10 分钟后" },
  { value: "1800", label: "30 分钟后" },
  { value: "3600", label: "1 小时后" },
];
const powerMode = ref("shutdown");
const powerPreset = ref("300");
const pending = ref<{ mode: string; remain: number } | null>(null);
const powerBusy = ref(false);
let countdown: number | undefined;

const modeLabel = computed(() =>
  ({ shutdown: "关机", restart: "重启", logoff: "注销" })[pending.value?.mode ?? ""] ?? ""
);

async function runPower() {
  if (powerBusy.value) return;
  powerBusy.value = true;
  const secs = Number(powerPreset.value);
  try {
    await scheduleShutdown(secs, powerMode.value);
    const label = modeLabel.value || (powerMode.value === "shutdown" ? "关机" : powerMode.value === "restart" ? "重启" : "注销");
    if (secs === 0) {
      toast(`正在执行${label}`, "warning");
    } else {
      pending.value = { mode: powerMode.value, remain: secs };
      toast(`已排定 ${secs / 60 >= 1 ? `${secs / 60} 分钟后` : `${secs} 秒后`}${label}`, "success");
    }
  } catch (err) {
    toast(String(err), "error");
  } finally {
    powerBusy.value = false;
  }
}

async function abortPower() {
  try {
    await cancelShutdown();
    pending.value = null;
    toast("已取消定时任务", "success");
  } catch (err) {
    toast(String(err), "error");
  }
}

function fmt(sec: number) {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

onMounted(() => {
  void loadPorts();
  countdown = window.setInterval(() => {
    if (pending.value && pending.value.remain > 0) pending.value.remain -= 1;
  }, 1000);
});

onUnmounted(() => {
  if (countdown !== undefined) window.clearInterval(countdown);
});
</script>

<template>
  <div>
    <PageHeader title="快捷工具" desc="看谁占了端口，一键打开系统面板，排定关机或重启。">
      <template #extra>
        <Button v-if="tab === 'ports'" variant="gray" size="sm" :disabled="portLoading" @click="loadPorts">
          <RefreshCw class="h-3.5 w-3.5" :class="portLoading && 'animate-spin'" />
          刷新
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

    <!-- 端口占用 -->
    <div v-show="tab === 'ports'" class="space-y-3">
      <div class="flex flex-wrap items-center gap-2">
        <div class="relative min-w-[200px] flex-1">
          <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
          <Input v-model="portFilter" placeholder="按端口、进程名或 PID 搜索" class="!pl-9" />
        </div>
        <Select v-model="portProto" :options="PROTO_OPTS" class="w-32" />
      </div>

      <Spinner v-if="portLoading" label="正在读取端口占用…" />
      <EmptyState
        v-else-if="!ports.length"
        icon="🔌"
        text="没有读到监听端口"
        sub="点右上角刷新重试。读取端口依赖系统网络命令模块。"
      />
      <Card v-else class="p-0">
        <div class="overflow-x-auto">
          <table class="w-full min-w-[620px] text-left text-xs">
            <thead>
              <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
                <th class="px-3 py-2.5 font-medium">协议</th>
                <th class="px-3 py-2.5 text-right font-medium">端口</th>
                <th class="px-3 py-2.5 font-medium">监听地址</th>
                <th class="px-3 py-2.5 font-medium">进程</th>
                <th class="px-3 py-2.5 text-right font-medium">PID</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-[var(--card-border)]">
              <tr v-for="p in shownPorts" :key="`${p.proto}-${p.port}-${p.pid}`" class="hover:bg-[var(--card-soft)]">
                <td class="px-3 py-2">
                  <span
                    class="rounded px-1.5 py-0.5 text-[10px] font-medium"
                    :class="p.proto === 'TCP' ? 'bg-[var(--primary-soft)] text-[var(--primary)]' : 'bg-[rgba(167,139,250,.16)] text-[#a78bfa]'"
                  >
                    {{ p.proto }}
                  </span>
                </td>
                <td class="px-3 py-2 text-right font-mono font-semibold tabular-nums">{{ p.port }}</td>
                <td class="px-3 py-2 font-mono text-[var(--muted)]">{{ p.addr || "—" }}</td>
                <td class="max-w-[200px] truncate px-3 py-2" :title="p.name">{{ p.name || "（已退出）" }}</td>
                <td class="px-3 py-2 text-right font-mono tabular-nums text-[var(--muted)]">{{ p.pid }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </Card>
      <p v-if="ports.length" class="text-[10px] text-[var(--muted)]">
        共 {{ ports.length }} 个监听端口，当前显示 {{ shownPorts.length }} 个。地址 <span class="font-mono">0.0.0.0</span> 表示对所有网卡开放。
      </p>
    </div>

    <!-- 系统入口 -->
    <div v-show="tab === 'tools'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
      <Card v-for="g in TOOL_GROUPS" :key="g.title" class="p-4">
        <p class="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Wrench class="h-4 w-4 text-[var(--primary)]" /> {{ g.title }}
        </p>
        <div class="grid grid-cols-2 gap-2">
          <button
            v-for="t in g.items"
            :key="t.id"
            type="button"
            :disabled="openingTool !== null"
            class="flex items-center justify-between gap-2 rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] px-2.5 py-2 text-left text-xs transition-colors hover:border-[var(--primary)] hover:text-[var(--primary)] disabled:opacity-50"
            @click="openTool(t.id)"
          >
            <span class="truncate">{{ t.label }}</span>
            <Loader2 v-if="openingTool === t.id" class="h-3 w-3 shrink-0 animate-spin" />
          </button>
        </div>
      </Card>
      <p class="text-[10px] text-[var(--muted)] md:col-span-2">
        全部走 Windows 自带的 .cpl / .msc 控制面板项，不调用任何第三方程序。注册表编辑器和用户账户会触发 UAC。
      </p>
    </div>

    <!-- 定时关机 -->
    <div v-show="tab === 'power'" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <Card class="p-5">
        <p class="mb-4 flex items-center gap-2 text-sm font-semibold">
          <Power class="h-4 w-4 text-[var(--primary)]" /> 排定电源任务
        </p>
        <div class="space-y-3">
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">动作</p>
            <Select v-model="powerMode" :options="POWER_OPTS" class="w-full" />
          </div>
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">延时</p>
            <Select v-model="powerPreset" :options="PRESET_OPTS" class="w-full" />
          </div>
          <Button variant="primary" class="w-full" :disabled="powerBusy" @click="runPower">
            <Loader2 v-if="powerBusy" class="h-4 w-4 animate-spin" />
            <AlarmClock v-else class="h-4 w-4" />
            执行
          </Button>
        </div>
        <p class="mt-3 text-[10px] text-[var(--muted)]">
          用 Windows 自带 shutdown 命令。定时期间会显示系统倒计时提示，可随时取消。
        </p>
      </Card>

      <Card class="p-5">
        <p class="mb-4 flex items-center gap-2 text-sm font-semibold">
          <Square class="h-4 w-4 text-[var(--muted)]" /> 已排定任务
        </p>
        <div v-if="pending" class="rounded-xl border border-[#f5a623]/40 bg-[#f5a623]/10 p-4 text-center">
          <p class="text-xs text-[var(--muted)]">将在</p>
          <p class="my-1 text-3xl font-bold tabular-nums text-[#f5a623]">{{ fmt(pending.remain) }}</p>
          <p class="text-xs text-[var(--muted)]">后{{ modeLabel }}</p>
          <Button variant="danger" size="sm" class="mt-3" @click="abortPower">
            <X class="h-3.5 w-3.5" /> 取消任务
          </Button>
        </div>
        <EmptyState v-else icon="⏱" text="没有排定的任务" sub="左侧选好动作和延时后点执行" />
      </Card>
    </div>
  </div>
</template>
