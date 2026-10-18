<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  Check, Download, Loader2, Pencil, Plus, RefreshCw, RotateCcw, Save, Search, Trash2,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Dialog from "../components/ui/Dialog.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import Switch from "../components/ui/Switch.vue";
import Checkbox from "../components/ui/Checkbox.vue";
import Progress from "../components/ui/Progress.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import Spinner from "../components/ui/Spinner.vue";
import {
  cleanJunk, driverList, exportDriverList, getEnvVars, listRuntimeLibs, listServices,
  listStartup, listDisabledStartup, listUwp, onRuntimeProgress, readHosts, removeUwp, installRuntime,
  scanJunk, setEnvVar, setService, toggleStartup, writeHosts,
  parseHosts, listHostsBackups, readHostsBackup,
  type DisabledStartup, type DriverInfo, type EnvVar, type HostEntry, type JunkItem,
  type RuntimeLib, type ServiceItem, type StartupItem, type UwpApp,
} from "../lib/tauri";
import { toast } from "../lib/toast";
import { formatBytes } from "../lib/utils";

/* ============ Tab 结构（8 个 Tab，懒加载 + 缓存 + 刷新） ============ */

const TABS = [
  { id: "startup", label: "启动项" },
  { id: "services", label: "服务" },
  { id: "hosts", label: "Hosts 编辑" },
  { id: "junk", label: "垃圾清理" },
  { id: "env", label: "环境变量" },
  { id: "runtime", label: "运行库" },
  { id: "uwp", label: "UWP 应用" },
  { id: "drivers", label: "驱动管理" },
] as const;

type TabId = (typeof TABS)[number]["id"];
const activeTab = ref<TabId>("startup");
const loaded = ref<Partial<Record<TabId, boolean>>>({});

function switchTab(id: TabId) {
  activeTab.value = id;
  if (!loaded.value[id]) void loadTab(id);
}

function loadTab(id: TabId): Promise<void> {
  switch (id) {
    case "startup": return loadStartup();
    case "services": return loadServices();
    case "hosts": return loadHosts();
    case "junk": return scanJunkNow();
    case "env": return loadEnv();
    case "runtime": return loadRuntimes();
    case "uwp": return loadUwp();
    case "drivers": return loadDrivers();
  }
}

/* ============ 1. 启动项 ============ */

const startupItems = ref<StartupItem[] | null>(null);
const startupLoading = ref(false);
const enabledMap = ref<Record<string, boolean>>({});
const toggling = ref<Record<string, boolean>>({});
/** 已被本工具禁用的项（配置目录里有备份，可原样还原） */
const disabledStartup = ref<DisabledStartup[]>([]);

async function loadStartup() {
  startupLoading.value = true;
  const [res, disabled] = await Promise.all([listStartup(), listDisabledStartup()]);
  startupLoading.value = false;
  if (res) {
    startupItems.value = res;
    const map: Record<string, boolean> = {};
    res.forEach((i) => {
      map[i.name] = enabledMap.value[i.name] ?? true;
    });
    enabledMap.value = map;
    loaded.value.startup = true;
  }
  disabledStartup.value = disabled ?? [];
}

async function onToggleStartup(item: StartupItem, enable: boolean) {
  toggling.value[item.name] = true;
  const res = await toggleStartup(item.name, enable);
  toggling.value[item.name] = false;
  if (res === null) {
    // 失败：回退开关状态
    enabledMap.value[item.name] = !enable;
    toast(`切换「${item.name}」失败`, "error");
  } else {
    enabledMap.value[item.name] = enable;
    toast(`已${enable ? "启用" : "禁用"}「${item.name}」`, "success");
    // 禁用后列表里会少一项，重新拉一次让「已禁用」区同步
    void loadStartup();
  }
}

/** 从「已禁用」区还原一项 */
async function restoreStartup(name: string) {
  toggling.value[name] = true;
  const res = await toggleStartup(name, true);
  toggling.value[name] = false;
  if (res === null) {
    toast(`还原「${name}」失败`, "error");
    return;
  }
  toast(`已还原「${name}」`, "success");
  await loadStartup();
}

function sourceLabel(src: string) {
  const map: Record<string, string> = {
    "hkcu-run": "当前用户启动项",
    "hklm-run": "系统启动项",
    "hklm-run-wow": "系统启动项 (32 位)",
    "startup-folder-user": "启动文件夹",
    "startup-folder-all": "公共启动文件夹",
  };
  return map[src] ?? src;
}

function fmtDate(ts: number) {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/* ============ 2. 服务 ============ */

const services = ref<ServiceItem[] | null>(null);
const svcLoading = ref(false);
const svcSearch = ref("");
const svcBusy = ref<string | null>(null);

async function loadServices() {
  svcLoading.value = true;
  const res = await listServices();
  svcLoading.value = false;
  if (res) {
    services.value = res;
    loaded.value.services = true;
  }
}

const filteredServices = computed(() => {
  const list = services.value ?? [];
  const q = svcSearch.value.trim().toLowerCase();
  let out = q
    ? list.filter((s) => `${s.name} ${s.display} ${s.status}`.toLowerCase().includes(q))
    : list.slice();
  if (svcRunningOnly.value) {
    out = out.filter((s) => svcStatusClass(s.status).includes("30a46c"));
  }
  if (svcAutoOnly.value) {
    out = out.filter((s) => s.start_type.toLowerCase().includes("auto"));
  }
  return out;
});

const svcRunningOnly = ref(false);
const svcAutoOnly = ref(false);

/** 当前筛选结果里有多少在运行 */
const svcRunningCount = computed(
  () => (services.value ?? []).filter((s) => svcStatusClass(s.status).includes("30a46c")).length
);

function svcStatusClass(status: string) {
  const v = status.toLowerCase();
  if (v.includes("running")) return "text-[#30a46c]";
  if (v.includes("stopped") || v.includes("paused")) return "text-[#e5484d]";
  return "text-[var(--muted)]";
}

async function svcAction(name: string, action: string) {
  if (svcBusy.value) return;
  svcBusy.value = name;
  const res = await setService(name, action);
  svcBusy.value = null;
  if (res !== null) toast(`服务「${name}」操作完成`, "success");
  await loadServices();
}

/* ============ 3. Hosts 编辑 ============ */

const hostsContent = ref("");
const hostsLoading = ref(false);
const hostsSaving = ref(false);
const hostsConfirm = ref(false);
/** structured = 条目视图，raw = 原始文本编辑 */
const hostsMode = ref<"structured" | "raw">("structured");
const hostEntries = ref<HostEntry[]>([]);
const hostsBackups = ref<string[]>([]);
const hostsBusy = ref(false);
const hostsNewIp = ref("127.0.0.1");
const hostsNewDomain = ref("");
const hostsRestoreFile = ref<string | null>(null);
const hostsShowBuiltin = ref(false);

/** 界面展示用：默认隐藏 localhost 示例行 */
const visibleHostEntries = computed(() => {
  const list = hostEntries.value.map((e, i) => ({ e, i }));
  return hostsShowBuiltin.value ? list : list.filter((x) => !x.e.builtin);
});
const builtinCount = computed(() => hostEntries.value.filter((e) => e.builtin).length);

async function loadHosts() {
  hostsLoading.value = true;
  const [raw, entries, backups] = await Promise.all([
    readHosts(),
    parseHosts(),
    listHostsBackups(),
  ]);
  hostsLoading.value = false;
  if (raw !== null) {
    hostsContent.value = raw;
    loaded.value.hosts = true;
  }
  hostEntries.value = entries ?? [];
  hostsBackups.value = backups ?? [];
}

async function saveHosts() {
  hostsSaving.value = true;
  const res = await writeHosts(hostsContent.value);
  hostsSaving.value = false;
  if (res !== null) {
    toast("hosts 文件已保存", "success");
    await loadHosts();
  }
}

/** 从磁盘重新载入并解析（放弃未保存修改） */
async function reloadHosts() {
  await loadHosts();
  toast("已重新载入 hosts", "info");
}

/** 把条目重建成 hosts 文本：保留原有注释块与空行，只改动条目行 */
function rebuildFromEntries() {
  const lines = hostsContent.value.split(/\r?\n/);
  const queue = hostEntries.value.slice();
  const out: string[] = [];
  for (const line of lines) {
    const t = line.trim();
    if (!t || (t.startsWith("#") && !looksLikeEntry(t))) {
      out.push(line);
      continue;
    }
    // 这是一条条目行：用队列里的下一条替换
    const e = queue.shift();
    if (!e) continue;
    const body = `${e.ip}\t${e.domains.join(" ")}`;
    out.push(e.enabled ? body : `# ${body}`);
  }
  // 剩下的（新增条目）追加到末尾
  for (const e of queue) {
    const body = `${e.ip}\t${e.domains.join(" ")}`;
    out.push(e.enabled ? body : `# ${body}`);
  }
  hostsContent.value = out.join("\n");
}

function looksLikeEntry(line: string) {
  const body = line.trim().replace(/^#+\s*/, "");
  const first = body.split(/\s+/)[0] ?? "";
  return first.length > 0 && first.split("").every((c) => /[0-9.:]/.test(c));
}

/** 切换某条启用状态（一行可能含多个域名） */
async function toggleHostEntry(idx: number, enable: boolean) {
  const e = hostEntries.value[idx];
  if (!e) return;
  hostEntries.value[idx] = { ...e, enabled: enable };
  rebuildFromEntries();
  hostsBusy.value = true;
  const res = await writeHosts(hostsContent.value);
  hostsBusy.value = false;
  if (res === null) {
    // 失败回滚界面状态
    hostEntries.value[idx] = e;
    rebuildFromEntries();
    return;
  }
  toast(`已${enable ? "启用" : "停用"} ${e.domains.join(", ")}`, "success");
}

/** 新增一条 hosts 记录 */
async function addHostEntry() {
  const ip = hostsNewIp.value.trim();
  const domain = hostsNewDomain.value.trim();
  if (!/^[0-9a-fA-F:.]+$/.test(ip)) {
    toast("IP 地址格式不正确", "warning");
    return;
  }
  if (!/^[a-zA-Z0-9._-]+$/.test(domain)) {
    toast("域名只能包含字母、数字、点、下划线和连字符", "warning");
    return;
  }
  const exists = hostEntries.value.some(
    (e) => e.ip === ip && e.domains.some((d) => d.toLowerCase() === domain.toLowerCase())
  );
  if (exists) {
    toast("这条记录已存在", "info");
    return;
  }
  hostEntries.value.push({ ip, domains: [domain], enabled: true, builtin: false });
  rebuildFromEntries();
  hostsBusy.value = true;
  const res = await writeHosts(hostsContent.value);
  hostsBusy.value = false;
  if (res === null) {
    hostEntries.value.pop();
    rebuildFromEntries();
    return;
  }
  hostsNewDomain.value = "";
  toast(`已添加 ${ip} → ${domain}`, "success");
  await loadHosts();
}

/** 删除一条记录 */
async function removeHostEntry(idx: number) {
  const e = hostEntries.value[idx];
  if (!e) return;
  hostEntries.value.splice(idx, 1);
  rebuildFromEntries();
  hostsBusy.value = true;
  const res = await writeHosts(hostsContent.value);
  hostsBusy.value = false;
  if (res === null) {
    await loadHosts();
    return;
  }
  toast(`已删除 ${e.domains.join(", ")}`, "success");
  await loadHosts();
}

/** 用某个备份覆盖当前 hosts */
async function restoreHostsBackup() {
  const file = hostsRestoreFile.value;
  if (!file) return;
  hostsBusy.value = true;
  const text = await readHostsBackup(file);
  if (text === null) {
    hostsBusy.value = false;
    return;
  }
  hostsContent.value = text;
  const res = await writeHosts(text);
  hostsBusy.value = false;
  hostsRestoreFile.value = null;
  if (res !== null) {
    toast("已从备份恢复 hosts", "success");
    await loadHosts();
  }
}

/* ============ 4. 垃圾清理 ============ */

const junkItems = ref<JunkItem[] | null>(null);
const junkScanning = ref(false);
const junkChecked = ref<Record<string, boolean>>({});
const junkCleaning = ref(false);
const junkConfirm = ref(false);

async function scanJunkNow() {
  junkScanning.value = true;
  const res = await scanJunk();
  junkScanning.value = false;
  if (res) {
    junkItems.value = res;
    const map: Record<string, boolean> = {};
    res.forEach((i) => {
      map[i.path] = true;
    });
    junkChecked.value = map;
    loaded.value.junk = true;
  }
}

const junkSelected = computed(() =>
  (junkItems.value ?? []).filter((i) => junkChecked.value[i.path])
);
const junkSelectedSize = computed(() =>
  junkSelected.value.reduce((s, i) => s + i.size, 0)
);

async function doCleanJunk() {
  if (junkSelected.value.length === 0) {
    toast("请先勾选要清理的项目", "info");
    return;
  }
  junkCleaning.value = true;
  const res = await cleanJunk(junkSelected.value.map((i) => i.path));
  junkCleaning.value = false;
  if (res !== null) {
    toast("垃圾清理完成", "success");
    await scanJunkNow();
  }
}

function toggleAllJunk(v: boolean) {
  const map: Record<string, boolean> = {};
  (junkItems.value ?? []).forEach((i) => {
    map[i.path] = v;
  });
  junkChecked.value = map;
}

/* ============ 5. 环境变量 ============ */

const envScope = ref<"user" | "system">("user");
const envVars = ref<EnvVar[] | null>(null);
const envLoading = ref(false);

async function loadEnv() {
  envLoading.value = true;
  const res = await getEnvVars(envScope.value);
  envLoading.value = false;
  if (res) {
    envVars.value = res;
    loaded.value.env = true;
  }
}

function switchEnvScope(s: "user" | "system") {
  if (envScope.value === s) return;
  envScope.value = s;
  void loadEnv();
}

// 编辑（双击或编辑按钮 → Dialog）
const envEditing = ref(false);
const envEditName = ref("");
const envEditValue = ref("");
const envSaving = ref(false);

function openEnvEdit(v: EnvVar) {
  envEditName.value = v.name;
  envEditValue.value = v.value;
  envEditing.value = true;
}

async function saveEnvEdit() {
  if (!envEditName.value.trim()) {
    toast("变量名不能为空", "warning");
    return;
  }
  envSaving.value = true;
  const res = await setEnvVar(envScope.value, envEditName.value, envEditValue.value, false);
  envSaving.value = false;
  if (res !== null) {
    envEditing.value = false;
    toast(`已保存「${envEditName.value}」`, "success");
    await loadEnv();
  }
}

// 删除（确认）
const envDelName = ref("");
const envDelConfirm = ref(false);

function openEnvDelete(name: string) {
  envDelName.value = name;
  envDelConfirm.value = true;
}

async function doEnvDelete() {
  const res = await setEnvVar(envScope.value, envDelName.value, "", true);
  if (res !== null) {
    toast(`已删除「${envDelName.value}」`, "success");
    await loadEnv();
  }
}

// 新增
const envAdding = ref(false);
const envAddName = ref("");
const envAddValue = ref("");
const envAddSaving = ref(false);

async function doEnvAdd() {
  if (!envAddName.value.trim()) {
    toast("变量名不能为空", "warning");
    return;
  }
  envAddSaving.value = true;
  const res = await setEnvVar(envScope.value, envAddName.value.trim(), envAddValue.value, false);
  envAddSaving.value = false;
  if (res !== null) {
    envAdding.value = false;
    envAddName.value = "";
    envAddValue.value = "";
    toast("变量已新增", "success");
    await loadEnv();
  }
}

/* ============ 6. 运行库 ============ */

const runtimes = ref<RuntimeLib[] | null>(null);
const rtLoading = ref(false);
const rtProgress = ref<Record<string, { stage: string; pct: number }>>({});
const rtInstalling = ref<Record<string, boolean>>({});

/** 已装 / 未装统计，顶部一眼看清缺什么 */
const rtInstalledCount = computed(() => (runtimes.value ?? []).filter((r) => r.installed).length);
const rtMissing = computed(() => (runtimes.value ?? []).filter((r) => !r.installed));
const rtAllInstalled = computed(
  () => runtimes.value !== null && runtimes.value.length > 0 && rtMissing.value.length === 0
);

/** 一键安装全部缺失运行库（串行，避免多个安装器互相抢锁） */
const rtBatchRunning = ref(false);
async function installMissingRuntimes() {
  const list = rtMissing.value;
  if (list.length === 0 || rtBatchRunning.value) return;
  rtBatchRunning.value = true;
  let ok = 0;
  let fail = 0;
  for (const lib of list) {
    rtInstalling.value[lib.id] = true;
    rtProgress.value[lib.id] = { stage: "准备中", pct: 0 };
    const res = await installRuntime(lib.id);
    delete rtInstalling.value[lib.id];
    delete rtProgress.value[lib.id];
    if (res === null) fail++;
    else ok++;
    await loadRuntimes();
  }
  rtBatchRunning.value = false;
  toast(
    `批量安装完成：成功 ${ok} 个${fail > 0 ? `，失败 ${fail} 个（可点单项重试或手动下载）` : ""}`,
    fail > 0 ? "warning" : "success"
  );
}

async function loadRuntimes() {
  rtLoading.value = true;
  const res = await listRuntimeLibs();
  rtLoading.value = false;
  if (res) {
    runtimes.value = res;
    loaded.value.runtime = true;
  }
}

async function doInstallRuntime(lib: RuntimeLib) {
  if (rtInstalling.value[lib.id]) return;
  rtInstalling.value[lib.id] = true;
  rtProgress.value[lib.id] = { stage: "准备中", pct: 0 };
  const res = await installRuntime(lib.id);
  delete rtInstalling.value[lib.id];
  delete rtProgress.value[lib.id];
  if (res === null) {
    toast(`「${lib.name}」自动安装失败，请前往官网手动下载：${lib.download_url || "见组件主页"}`, "error");
  } else {
    toast(`「${lib.name}」安装完成`, "success");
  }
  await loadRuntimes();
}

/* ============ 7. UWP 应用 ============ */

const uwpApps = ref<UwpApp[] | null>(null);
const uwpLoading = ref(false);
const uwpConfirmApp = ref<UwpApp | null>(null);

async function loadUwp() {
  uwpLoading.value = true;
  const res = await listUwp();
  uwpLoading.value = false;
  if (res) {
    uwpApps.value = res;
    loaded.value.uwp = true;
  }
}

async function doRemoveUwp() {
  const app = uwpConfirmApp.value;
  if (!app) return;
  uwpLoading.value = true;
  const res = await removeUwp(app.package);
  uwpLoading.value = false;
  if (res !== null) {
    toast(`已移除「${app.name}」`, "success");
    await loadUwp();
  }
}

/* ============ 8. 驱动管理 ============ */

const drivers = ref<DriverInfo[] | null>(null);
const driversLoading = ref(false);
const driverSearch = ref("");
const exporting = ref(false);

async function loadDrivers() {
  driversLoading.value = true;
  const res = await driverList();
  driversLoading.value = false;
  if (res) {
    drivers.value = res;
    loaded.value.drivers = true;
  }
}

const filteredDrivers = computed(() => {
  const list = drivers.value ?? [];
  const q = driverSearch.value.trim().toLowerCase();
  if (!q) return list;
  return list.filter((d) =>
    `${d.device_name} ${d.provider} ${d.inf_name}`.toLowerCase().includes(q)
  );
});

async function doExportDrivers() {
  if (drivers.value === null || drivers.value.length === 0) {
    toast("请先加载驱动列表", "info");
    return;
  }
  exporting.value = true;
  const res = await exportDriverList();
  exporting.value = false;
  if (res) toast(`驱动清单已导出（${res.count} 项）：${res.file}`, "success");
}

/* ============ 生命周期：运行库进度事件 ============ */

let unRuntime: UnlistenFn | undefined;

onMounted(async () => {
  unRuntime = await onRuntimeProgress((p) => {
    // 仅更新当前页面发起的安装任务（按 id 过滤）
    if (rtProgress.value[p.id]) {
      rtProgress.value[p.id] = { stage: p.stage, pct: p.pct };
    }
  });
});

onUnmounted(() => {
  unRuntime?.();
});
</script>

<template>
  <div>
    <PageHeader title="系统工具" desc="启动项、服务、Hosts、垃圾清理、环境变量、运行库、UWP 与驱动管理" />

    <!-- Tab 栏（圆角标签风格） -->
    <div class="mb-5 flex flex-wrap gap-1.5 rounded-2xl border border-[var(--card-border)] bg-[var(--card-solid)]/60 p-1.5 backdrop-blur">
      <button
        v-for="t in TABS"
        :key="t.id"
        class="cursor-pointer rounded-xl px-3.5 py-1.5 text-sm transition-colors"
        :class="
          activeTab === t.id
            ? 'bg-[var(--primary)] font-medium text-white shadow'
            : 'text-[var(--muted)] hover:bg-[var(--btn-gray)] hover:text-[var(--text)]'
        "
        @click="switchTab(t.id)"
      >
        {{ t.label }}
      </button>
    </div>

    <!-- ===== 1. 启动项 ===== -->
    <Card v-if="activeTab === 'startup'" class="p-5">
      <div class="mb-4 flex items-center justify-between">
        <h4 class="font-semibold">开机启动项</h4>
        <Button variant="gray" size="icon" title="刷新" :disabled="startupLoading" @click="loadStartup">
          <RefreshCw class="h-4 w-4" :class="startupLoading && 'animate-spin'" />
        </Button>
      </div>

      <Spinner v-if="startupLoading" label="正在读取启动项…" />
      <EmptyState v-else-if="!startupItems" icon="🚀" text="尚未加载启动项" sub="点击右上角刷新按钮加载" />
      <EmptyState v-else-if="startupItems.length === 0" icon="🚀" text="没有发现启动项" />
      <div v-else class="overflow-x-auto rounded-xl border border-[var(--card-border)]">
        <table class="w-full min-w-[680px] text-left text-xs">
          <thead>
            <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
              <th class="px-3 py-2.5 font-medium">名称</th>
              <th class="px-3 py-2.5 font-medium">命令</th>
              <th class="px-3 py-2.5 font-medium">位置</th>
              <th class="px-3 py-2.5 text-right font-medium">启用</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-[var(--card-border)]">
            <tr v-for="(s, i) in startupItems" :key="`${s.name}-${i}`" class="transition-colors hover:bg-[var(--card-soft)]">
              <td class="max-w-[180px] truncate px-3 py-2.5 font-medium text-[var(--text-strong)]" :title="s.name">{{ s.name || "—" }}</td>
              <td class="max-w-[260px] truncate px-3 py-2.5 font-mono" :title="s.command">{{ s.command || "—" }}</td>
              <td class="max-w-[180px] truncate px-3 py-2.5" :title="s.location">{{ s.location || "—" }}</td>
              <td class="px-3 py-2.5 text-right">
                <Switch
                  :checked="enabledMap[s.name] ?? true"
                  :disabled="toggling[s.name]"
                  class="ml-auto"
                  @update:checked="(v: boolean) => onToggleStartup(s, v)"
                />
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 已禁用区：本工具禁用的项都有备份，可以原样还原 -->
      <div v-if="disabledStartup.length" class="mt-4 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
        <div class="mb-2 flex flex-wrap items-center gap-2">
          <p class="text-xs font-semibold text-[var(--text-strong)]">已禁用（{{ disabledStartup.length }}）</p>
          <p class="text-[10px] text-[var(--muted)]">原始命令已备份到配置目录，还原会写回原位置</p>
        </div>
        <div class="space-y-1.5">
          <div
            v-for="d in disabledStartup"
            :key="`${d.name}-${d.source}`"
            class="flex flex-wrap items-center gap-2 rounded-lg bg-[var(--card)] px-2.5 py-1.5"
          >
            <span class="min-w-0 flex-1 truncate text-xs font-medium" :title="d.name">{{ d.name }}</span>
            <span class="shrink-0 rounded bg-[var(--btn-gray)] px-1.5 py-0.5 text-[10px] text-[var(--muted)]">
              {{ sourceLabel(d.source) }}
            </span>
            <span class="hidden shrink-0 text-[10px] text-[var(--muted)] sm:inline">{{ fmtDate(d.disabled_at) }}</span>
            <Button
              variant="gray"
              size="sm"
              :disabled="toggling[d.name]"
              title="还原到原位置"
              @click="restoreStartup(d.name)"
            >
              还原
            </Button>
          </div>
        </div>
      </div>
    </Card>

    <!-- ===== 2. 服务 ===== -->
    <Card v-else-if="activeTab === 'services'" class="p-5">
      <div class="mb-4 flex flex-wrap items-center gap-3">
        <div class="relative min-w-[220px] flex-1">
          <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
          <Input v-model="svcSearch" placeholder="搜索服务名称、显示名或状态…" :disabled="services === null" class="!pl-9" />
        </div>
        <Button variant="gray" size="sm" :disabled="svcLoading" @click="loadServices">
          <RefreshCw class="h-3.5 w-3.5" :class="svcLoading && 'animate-spin'" /> 刷新
        </Button>
      </div>

      <!-- 快捷筛选 -->
      <div v-if="services" class="mb-3 flex flex-wrap items-center gap-2 text-xs">
        <button
          type="button"
          class="rounded-full px-2.5 py-1 font-medium transition-colors"
          :class="svcRunningOnly ? 'bg-[#30a46c] text-white' : 'bg-[#30a46c]/12 text-[#30a46c]'"
          @click="svcRunningOnly = !svcRunningOnly"
        >
          只看运行中 {{ svcRunningCount }}
        </button>
        <button
          type="button"
          class="rounded-full px-2.5 py-1 font-medium transition-colors"
          :class="svcAutoOnly ? 'bg-[var(--primary)] text-white' : 'bg-[var(--primary-soft)] text-[var(--primary)]'"
          @click="svcAutoOnly = !svcAutoOnly"
        >
          只看自动启动
        </button>
        <button
          v-if="svcRunningOnly || svcAutoOnly"
          type="button"
          class="rounded-full bg-[var(--btn-gray)] px-2.5 py-1 text-[var(--muted)]"
          @click="svcRunningOnly = false; svcAutoOnly = false"
        >
          清除筛选
        </button>
      </div>

      <Spinner v-if="svcLoading" label="正在枚举系统服务…" />
      <EmptyState v-else-if="!services" icon="⚙️" text="尚未加载服务列表" sub="点击「刷新」枚举本机系统服务（操作需管理员权限）" />
      <EmptyState v-else-if="filteredServices.length === 0" icon="🔍" text="没有匹配的服务" sub="试试调整搜索词" />
      <template v-else>
        <p class="mb-2 text-xs text-[var(--muted)]">
          共 {{ services.length }} 个服务，当前显示 {{ filteredServices.length }} 个
        </p>
        <div class="max-h-[560px] overflow-auto rounded-xl border border-[var(--card-border)]">
          <table class="w-full min-w-[720px] text-left text-xs">
            <thead class="sticky top-0 z-10">
              <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)] backdrop-blur">
                <th class="px-3 py-2.5 font-medium">名称</th>
                <th class="px-3 py-2.5 font-medium">显示名</th>
                <th class="px-3 py-2.5 font-medium">状态</th>
                <th class="px-3 py-2.5 font-medium">启动类型</th>
                <th class="px-3 py-2.5 text-right font-medium">操作</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-[var(--card-border)]">
              <tr v-for="(s, i) in filteredServices" :key="`${s.name}-${i}`" class="transition-colors hover:bg-[var(--card-soft)]">
                <td class="max-w-[160px] truncate px-3 py-2.5 font-mono font-medium text-[var(--text-strong)]" :title="s.name">{{ s.name || "—" }}</td>
                <td class="max-w-[220px] truncate px-3 py-2.5" :title="s.display">{{ s.display || "—" }}</td>
                <td class="px-3 py-2.5 font-semibold" :class="svcStatusClass(s.status)">{{ s.status || "—" }}</td>
                <td class="px-3 py-2.5">{{ s.start_type || "—" }}</td>
                <td class="px-3 py-2.5">
                  <div class="flex justify-end gap-1.5">
                    <Button variant="gray" size="sm" :disabled="svcBusy !== null" @click="svcAction(s.name, 'start')">启动</Button>
                    <Button variant="gray" size="sm" :disabled="svcBusy !== null" @click="svcAction(s.name, 'stop')">停止</Button>
                    <Button variant="gray" size="sm" :disabled="svcBusy !== null" @click="svcAction(s.name, 'auto')">自动</Button>
                    <Button variant="danger" size="sm" :disabled="svcBusy !== null" @click="svcAction(s.name, 'disabled')">禁用</Button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>
    </Card>

    <!-- ===== 3. Hosts 编辑 ===== -->
    <Card v-else-if="activeTab === 'hosts'" class="p-5">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
        <div class="flex flex-wrap items-center gap-2">
          <h4 class="font-semibold">Hosts 文件</h4>
          <span class="text-xs text-[var(--muted)]">
            {{ hostEntries.length }} 条记录 · {{ hostEntries.filter((e) => e.enabled).length }} 条生效
          </span>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <div class="flex gap-1 rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] p-1">
            <button
              v-for="m in [{ v: 'structured', l: '条目视图' }, { v: 'raw', l: '原始文本' }]"
              :key="m.v"
              type="button"
              class="rounded px-2 py-1 text-xs"
              :class="hostsMode === m.v ? 'bg-[var(--primary)] text-white' : 'text-[var(--muted)]'"
              @click="hostsMode = m.v as never"
            >
              {{ m.l }}
            </button>
          </div>
          <Button variant="gray" size="sm" :disabled="hostsLoading" @click="reloadHosts">
            <RotateCcw class="h-3.5 w-3.5" /> 重新载入
          </Button>
          <Button
            v-if="hostsMode === 'raw'"
            variant="primary"
            size="sm"
            :disabled="hostsLoading || hostsSaving"
            @click="hostsConfirm = true"
          >
            <Save class="h-3.5 w-3.5" /> 保存
          </Button>
        </div>
      </div>

      <Spinner v-if="hostsLoading" label="正在读取 hosts 文件…" />
      <template v-else>
        <!-- 条目视图 -->
        <template v-if="hostsMode === 'structured'">
          <div class="mb-3 flex flex-wrap items-center gap-2 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-2.5">
            <Input v-model="hostsNewIp" placeholder="IP" class="w-32" />
            <span class="text-xs text-[var(--muted)]">→</span>
            <div class="min-w-[160px] flex-1">
              <Input v-model="hostsNewDomain" placeholder="域名，如 example.com" @keyup.enter="addHostEntry" />
            </div>
            <Button variant="primary" size="sm" :disabled="hostsBusy" @click="addHostEntry">
              <Plus class="h-3.5 w-3.5" /> 添加
            </Button>
          </div>

          <EmptyState
            v-if="!visibleHostEntries.length"
            icon="🌐"
            text="hosts 里还没有自定义记录"
            sub="上面填 IP 和域名即可添加；默认只有注释说明"
          />
          <div v-else class="max-h-[440px] divide-y divide-[var(--card-border)] overflow-y-auto rounded-xl border border-[var(--card-border)]">
            <div
              v-for="{ e, i } in visibleHostEntries"
              :key="`${e.ip}-${i}`"
              class="flex flex-wrap items-center gap-2 px-3 py-2 text-xs transition-colors hover:bg-[var(--card-soft)]"
              :class="!e.enabled && 'opacity-60'"
            >
              <Switch
                :checked="e.enabled"
                :disabled="hostsBusy"
                class="scale-90"
                @update:checked="(v: boolean) => toggleHostEntry(i, v)"
              />
              <span class="shrink-0 rounded bg-[var(--btn-gray)] px-1.5 py-0.5 font-mono text-[10px]">{{ e.ip }}</span>
              <span class="min-w-0 flex-1 truncate font-mono" :title="e.domains.join(' ')">
                {{ e.domains.join(" · ") }}
              </span>
              <span v-if="e.builtin" class="shrink-0 text-[10px] text-[var(--muted)]">系统示例</span>
              <Button
                variant="ghost"
                size="sm"
                class="text-[var(--muted)] hover:text-[#e5484d]"
                :disabled="hostsBusy"
                title="删除这条记录"
                @click="removeHostEntry(i)"
              >
                <Trash2 class="h-3 w-3" />
              </Button>
            </div>
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-2">
            <Button
              v-if="builtinCount"
              variant="ghost"
              size="sm"
              class="text-[var(--muted)]"
              @click="hostsShowBuiltin = !hostsShowBuiltin"
            >
              {{ hostsShowBuiltin ? "隐藏" : "显示" }}系统示例（{{ builtinCount }}）
            </Button>
            <p class="text-xs text-[var(--muted)]">
              开关直接写入系统 hosts（需管理员权限），每次写入前会自动备份原文件。
            </p>
          </div>
        </template>

        <!-- 原始文本 -->
        <template v-else>
          <textarea
            v-model="hostsContent"
            rows="18"
            spellcheck="false"
            class="input-base w-full resize-y font-mono text-xs leading-relaxed"
            placeholder="# 加载失败或文件为空，可点击「重新载入」重试"
          />
          <p class="mt-2 text-xs text-[var(--muted)]">
            保存需管理员权限；「重新载入」会放弃当前修改并从磁盘重读
          </p>
        </template>

        <!-- 备份回滚 -->
        <div class="mt-4 border-t border-[var(--card-border)] pt-3">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-xs font-medium text-[var(--text-strong)]">历史备份</span>
            <span class="text-[10px] text-[var(--muted)]">{{ hostsBackups.length }} 份</span>
            <div class="ml-auto flex items-center gap-2">
              <Select
                :model-value="hostsRestoreFile ?? ''"
                :options="hostsBackups.map((f) => ({ value: f, label: f }))"
                placeholder="选择备份文件"
                class="w-56"
                :disabled="!hostsBackups.length"
                @update:model-value="(v: string) => (hostsRestoreFile = v || null)"
              />
              <Button
                variant="gray"
                size="sm"
                :disabled="!hostsRestoreFile || hostsBusy"
                @click="restoreHostsBackup"
              >
                <RotateCcw class="h-3.5 w-3.5" /> 恢复此备份
              </Button>
            </div>
          </div>
          <p v-if="!hostsBackups.length" class="mt-1.5 text-[10px] text-[var(--muted)]">
            还没有备份。首次保存 hosts 后会自动在配置目录留一份原文件。
          </p>
        </div>
      </template>

      <ConfirmDialog
        v-model:open="hostsConfirm"
        title="保存 hosts 文件"
        message="即将把当前编辑内容写入系统 hosts 文件（需管理员权限）。该文件影响全局域名解析，确定保存吗？"
        confirm-text="保存"
        @confirm="saveHosts"
      />
    </Card>

    <!-- ===== 4. 垃圾清理 ===== -->
    <Card v-else-if="activeTab === 'junk'" class="p-5">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
        <h4 class="font-semibold">垃圾文件清理</h4>
        <div class="flex items-center gap-2">
          <Button variant="gray" size="sm" :disabled="junkScanning" @click="scanJunkNow">
            <RefreshCw class="h-3.5 w-3.5" :class="junkScanning && 'animate-spin'" />
            {{ junkItems ? "重新扫描" : "扫描垃圾" }}
          </Button>
          <Button
            variant="danger"
            size="sm"
            :disabled="junkScanning || junkCleaning || junkSelected.length === 0"
            @click="junkConfirm = true"
          >
            <Loader2 v-if="junkCleaning" class="h-3.5 w-3.5 animate-spin" />
            <Trash2 v-else class="h-3.5 w-3.5" />
            清理已选（{{ junkSelected.length }} 项 / {{ formatBytes(junkSelectedSize) }}）
          </Button>
        </div>
      </div>

      <Spinner v-if="junkScanning" label="正在扫描垃圾文件…" />
      <EmptyState v-else-if="!junkItems" icon="🧹" text="尚未扫描" sub="点击「扫描垃圾」查找可安全清理的临时文件" />
      <EmptyState v-else-if="junkItems.length === 0" icon="✨" text="很干净，没有发现可清理的垃圾" />
      <template v-else>
        <div class="mb-2 flex items-center gap-2 text-xs text-[var(--muted)]">
          <Checkbox :checked="junkSelected.length === junkItems.length" @update:checked="toggleAllJunk" />
          全选 / 共发现 {{ junkItems.length }} 项
        </div>
        <div class="max-h-[480px] overflow-auto rounded-xl border border-[var(--card-border)]">
          <table class="w-full min-w-[680px] text-left text-xs">
            <thead class="sticky top-0 z-10">
              <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)] backdrop-blur">
                <th class="w-10 px-3 py-2.5" />
                <th class="px-3 py-2.5 font-medium">路径</th>
                <th class="px-3 py-2.5 text-right font-medium">大小</th>
                <th class="px-3 py-2.5 font-medium">类型</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-[var(--card-border)]">
              <tr v-for="(j, i) in junkItems" :key="`${j.path}-${i}`" class="transition-colors hover:bg-[var(--card-soft)]">
                <td class="px-3 py-2.5">
                  <Checkbox v-model:checked="junkChecked[j.path]" />
                </td>
                <td class="max-w-[420px] truncate px-3 py-2.5 font-mono" :title="j.path">{{ j.path }}</td>
                <td class="px-3 py-2.5 text-right tabular-nums">{{ formatBytes(j.size) }}</td>
                <td class="px-3 py-2.5">{{ j.kind }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>

      <ConfirmDialog
        v-model:open="junkConfirm"
        title="清理垃圾文件"
        :message="`即将删除已选的 ${junkSelected.length} 项（共 ${formatBytes(junkSelectedSize)}）。删除后不可恢复，确定继续吗？`"
        confirm-text="清理"
        danger
        @confirm="doCleanJunk"
      />
    </Card>

    <!-- ===== 5. 环境变量 ===== -->
    <Card v-else-if="activeTab === 'env'" class="p-5">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-1.5 rounded-xl border border-[var(--card-border)] bg-[var(--card-solid)]/60 p-1">
          <button
            class="cursor-pointer rounded-lg px-3 py-1 text-xs transition-colors"
            :class="envScope === 'user' ? 'bg-[var(--primary)] font-medium text-white' : 'text-[var(--muted)] hover:bg-[var(--btn-gray)]'"
            @click="switchEnvScope('user')"
          >用户变量</button>
          <button
            class="cursor-pointer rounded-lg px-3 py-1 text-xs transition-colors"
            :class="envScope === 'system' ? 'bg-[var(--primary)] font-medium text-white' : 'text-[var(--muted)] hover:bg-[var(--btn-gray)]'"
            @click="switchEnvScope('system')"
          >系统变量</button>
        </div>
        <div class="flex items-center gap-2">
          <Button variant="primary" size="sm" @click="envAdding = true">
            <Plus class="h-3.5 w-3.5" /> 新增变量
          </Button>
          <Button variant="gray" size="sm" :disabled="envLoading" @click="loadEnv">
            <RefreshCw class="h-3.5 w-3.5" :class="envLoading && 'animate-spin'" /> 刷新
          </Button>
        </div>
      </div>

      <Spinner v-if="envLoading" label="正在读取环境变量…" />
      <EmptyState v-else-if="!envVars" icon="🧬" text="尚未加载环境变量" sub="点击「刷新」读取当前作用域的变量列表" />
      <EmptyState v-else-if="envVars.length === 0" icon="🧬" text="当前作用域没有环境变量" />
      <div v-else class="max-h-[560px] overflow-auto rounded-xl border border-[var(--card-border)]">
        <table class="w-full min-w-[620px] text-left text-xs">
          <thead class="sticky top-0 z-10">
            <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)] backdrop-blur">
              <th class="px-3 py-2.5 font-medium">名称</th>
              <th class="px-3 py-2.5 font-medium">值（双击可编辑）</th>
              <th class="px-3 py-2.5 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-[var(--card-border)]">
            <tr v-for="(v, i) in envVars" :key="`${v.name}-${i}`" class="transition-colors hover:bg-[var(--card-soft)]">
              <td class="max-w-[200px] truncate px-3 py-2.5 font-mono font-medium text-[var(--text-strong)]" :title="v.name">{{ v.name }}</td>
              <td class="max-w-[320px] cursor-text truncate px-3 py-2.5 font-mono" :title="v.value" @dblclick="openEnvEdit(v)">{{ v.value }}</td>
              <td class="px-3 py-2.5">
                <div class="flex justify-end gap-1.5">
                  <Button variant="gray" size="icon" title="编辑" @click="openEnvEdit(v)">
                    <Pencil class="h-3.5 w-3.5" />
                  </Button>
                  <Button variant="danger" size="icon" title="删除" @click="openEnvDelete(v.name)">
                    <Trash2 class="h-3.5 w-3.5" />
                  </Button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 编辑弹窗 -->
      <Dialog v-model:open="envEditing" title="编辑环境变量">
        <div class="space-y-3">
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">变量名</p>
            <Input v-model="envEditName" disabled class="font-mono" />
          </div>
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">变量值</p>
            <Input v-model="envEditValue" class="font-mono" placeholder="输入新的变量值" />
          </div>
        </div>
        <template #footer>
          <div class="flex justify-end gap-3">
            <Button variant="gray" @click="envEditing = false">取消</Button>
            <Button variant="primary" :disabled="envSaving" @click="saveEnvEdit">
              <Loader2 v-if="envSaving" class="h-3.5 w-3.5 animate-spin" />
              保存
            </Button>
          </div>
        </template>
      </Dialog>

      <!-- 删除确认 -->
      <ConfirmDialog
        v-model:open="envDelConfirm"
        title="删除环境变量"
        :message="`即将删除${envScope === 'user' ? '用户' : '系统'}变量「${envDelName}」。此操作影响系统行为，确定删除吗？`"
        confirm-text="删除"
        danger
        @confirm="doEnvDelete"
      />

      <!-- 新增弹窗 -->
      <Dialog v-model:open="envAdding" :title="`新增${envScope === 'user' ? '用户' : '系统'}环境变量`">
        <div class="space-y-3">
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">变量名</p>
            <Input v-model="envAddName" class="font-mono" placeholder="例如 JAVA_HOME" />
          </div>
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">变量值</p>
            <Input v-model="envAddValue" class="font-mono" placeholder="变量值" />
          </div>
        </div>
        <template #footer>
          <div class="flex justify-end gap-3">
            <Button variant="gray" @click="envAdding = false">取消</Button>
            <Button variant="primary" :disabled="envAddSaving" @click="doEnvAdd">
              <Loader2 v-if="envAddSaving" class="h-3.5 w-3.5 animate-spin" />
              新增
            </Button>
          </div>
        </template>
      </Dialog>
    </Card>

    <!-- ===== 6. 运行库 ===== -->
    <Card v-else-if="activeTab === 'runtime'" class="p-5">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
        <div class="flex flex-wrap items-center gap-2">
          <h4 class="font-semibold">常用运行库</h4>
          <span v-if="runtimes" class="text-xs text-[var(--muted)]">
            {{ rtInstalledCount }} / {{ runtimes.length }} 已安装
          </span>
        </div>
        <div class="flex items-center gap-2">
          <Button
            v-if="rtMissing.length"
            variant="primary"
            size="sm"
            :disabled="rtBatchRunning || rtLoading"
            :title="`依次安装 ${rtMissing.length} 个缺失运行库`"
            @click="installMissingRuntimes"
          >
            <Loader2 v-if="rtBatchRunning" class="h-3.5 w-3.5 animate-spin" />
            <Download v-else class="h-3.5 w-3.5" />
            一键装齐（{{ rtMissing.length }}）
          </Button>
          <Button variant="gray" size="sm" :disabled="rtLoading || rtBatchRunning" @click="loadRuntimes">
            <RefreshCw class="h-3.5 w-3.5" :class="rtLoading && 'animate-spin'" /> 刷新
          </Button>
        </div>
      </div>

      <!-- 缺失提示条 -->
      <div
        v-if="rtMissing.length"
        class="mb-3 rounded-lg border border-[#f5a623]/40 bg-[#f5a623]/10 px-3 py-2 text-xs text-[#b7791f]"
      >
        有 {{ rtMissing.length }} 个运行库未安装：{{ rtMissing.map((r) => r.name).join("、") }}。缺少时部分软件会报「找不到 DLL」或直接启动失败。
      </div>
      <div
        v-else-if="rtAllInstalled"
        class="mb-3 rounded-lg border border-[#30a46c]/40 bg-[#30a46c]/10 px-3 py-2 text-xs text-[#30a46c]"
      >
        常用运行库均已就绪。
      </div>

      <Spinner v-if="rtLoading" label="正在检测运行库…" />
      <EmptyState v-else-if="!runtimes" icon="🧩" text="尚未检测运行库" sub="点击「刷新」检测本机常用运行库安装状态" />
      <EmptyState v-else-if="runtimes.length === 0" icon="🧩" text="没有可管理的运行库" />
      <div v-else class="space-y-3">
        <div
          v-for="r in runtimes"
          :key="r.id"
          class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-4"
        >
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div class="min-w-0">
              <p class="flex items-center gap-2 text-sm font-semibold text-[var(--text-strong)]">
                {{ r.name }}
                <span
                  class="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] font-medium"
                  :class="r.installed ? 'bg-[#30a46c]/15 text-[#30a46c]' : 'bg-[var(--btn-gray)] text-[var(--muted)]'"
                >
                  <Check v-if="r.installed" class="h-3 w-3" :stroke-width="3" />
                  {{ r.installed ? "已安装" : "未安装" }}
                </span>
              </p>
              <p class="mt-0.5 truncate text-xs text-[var(--muted)]">
                {{ r.version ? `版本 ${r.version}` : "未检测到版本信息" }}
              </p>
            </div>
            <Button
              v-if="!r.installed"
              variant="primary"
              size="sm"
              :disabled="rtInstalling[r.id]"
              @click="doInstallRuntime(r)"
            >
              <Loader2 v-if="rtInstalling[r.id]" class="h-3.5 w-3.5 animate-spin" />
              <Download v-else class="h-3.5 w-3.5" />
              {{ rtInstalling[r.id] ? "安装中…" : "安装" }}
            </Button>
          </div>
          <!-- 按 id 过滤显示的安装进度 -->
          <div v-if="rtProgress[r.id]" class="mt-3">
            <div class="mb-1 flex items-center justify-between text-xs text-[var(--muted)]">
              <span>{{ rtProgress[r.id].stage }}</span>
              <span class="tabular-nums">{{ Math.round(rtProgress[r.id].pct) }}%</span>
            </div>
            <Progress :value="rtProgress[r.id].pct" />
          </div>
        </div>
      </div>
    </Card>

    <!-- ===== 7. UWP 应用 ===== -->
    <Card v-else-if="activeTab === 'uwp'" class="p-5">
      <div class="mb-4 flex items-center justify-between">
        <h4 class="font-semibold">UWP 应用</h4>
        <Button variant="gray" size="sm" :disabled="uwpLoading" @click="loadUwp">
          <RefreshCw class="h-3.5 w-3.5" :class="uwpLoading && 'animate-spin'" /> 刷新
        </Button>
      </div>

      <Spinner v-if="uwpLoading" label="正在枚举 UWP 应用…" />
      <EmptyState v-else-if="!uwpApps" icon="🪟" text="尚未加载 UWP 应用" sub="点击「刷新」枚举本机 UWP 应用包" />
      <EmptyState v-else-if="uwpApps.length === 0" icon="🪟" text="未发现 UWP 应用" />
      <div v-else class="max-h-[560px] overflow-auto rounded-xl border border-[var(--card-border)]">
        <table class="w-full min-w-[680px] text-left text-xs">
          <thead class="sticky top-0 z-10">
            <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)] backdrop-blur">
              <th class="px-3 py-2.5 font-medium">名称</th>
              <th class="px-3 py-2.5 font-medium">版本</th>
              <th class="px-3 py-2.5 font-medium">包名</th>
              <th class="px-3 py-2.5 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-[var(--card-border)]">
            <tr v-for="(u, i) in uwpApps" :key="`${u.package}-${i}`" class="transition-colors hover:bg-[var(--card-soft)]">
              <td class="max-w-[200px] truncate px-3 py-2.5 font-medium text-[var(--text-strong)]" :title="u.name">{{ u.name || "—" }}</td>
              <td class="px-3 py-2.5 font-mono">{{ u.version || "—" }}</td>
              <td class="max-w-[280px] truncate px-3 py-2.5 font-mono" :title="u.package">{{ u.package || "—" }}</td>
              <td class="px-3 py-2.5 text-right">
                <Button variant="danger" size="sm" :disabled="uwpLoading" @click="uwpConfirmApp = u">移除</Button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <ConfirmDialog
        :open="uwpConfirmApp !== null"
        title="移除 UWP 应用"
        :message="`即将移除 UWP 应用「${uwpConfirmApp?.name ?? ''}」（${uwpConfirmApp?.package ?? ''}）。该操作不可逆，确定移除吗？`"
        confirm-text="移除"
        danger
        @update:open="(v: boolean) => { if (!v) uwpConfirmApp = null; }"
        @confirm="doRemoveUwp"
      />
    </Card>

    <!-- ===== 8. 驱动管理 ===== -->
    <Card v-else-if="activeTab === 'drivers'" class="p-5">
      <div class="mb-4 flex flex-wrap items-center gap-3">
        <div class="relative min-w-[220px] flex-1">
          <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
          <Input v-model="driverSearch" placeholder="搜索设备名、提供商或 INF…" :disabled="drivers === null" class="!pl-9" />
        </div>
        <Button variant="primary" size="sm" :disabled="driversLoading" @click="loadDrivers">
          <Loader2 v-if="driversLoading" class="h-3.5 w-3.5 animate-spin" />
          <RefreshCw v-else class="h-3.5 w-3.5" />
          {{ drivers === null ? "加载驱动列表" : "重新加载" }}
        </Button>
        <Button
          variant="gray"
          size="sm"
          :disabled="exporting || drivers === null || drivers.length === 0"
          @click="doExportDrivers"
        >
          <Loader2 v-if="exporting" class="h-3.5 w-3.5 animate-spin" />
          <Download v-else class="h-3.5 w-3.5" />
          导出清单 CSV
        </Button>
      </div>

      <!-- 加载中 -->
      <Spinner v-if="driversLoading" label="正在枚举本机驱动…" />

      <!-- 未加载 -->
      <EmptyState
        v-else-if="drivers === null"
        icon="🧩"
        text="尚未加载驱动列表"
        sub="点击「加载驱动列表」枚举本机已安装驱动"
      />

      <!-- 加载为空 -->
      <EmptyState
        v-else-if="drivers.length === 0"
        icon="🧩"
        text="未获取到驱动信息"
        sub="可能需要管理员权限，或系统暂未返回驱动列表"
      />

      <!-- 搜索无结果 -->
      <EmptyState
        v-else-if="filteredDrivers.length === 0"
        icon="🔍"
        text="没有匹配的驱动"
        sub="试试调整搜索词"
      />

      <!-- 驱动表格 -->
      <template v-else>
        <p class="mb-2 text-xs text-[var(--muted)]">
          共 {{ drivers.length }} 个驱动，当前显示 {{ filteredDrivers.length }} 个
        </p>
        <div class="max-h-[560px] overflow-auto rounded-xl border border-[var(--card-border)]">
          <table class="w-full min-w-[680px] text-left text-xs">
            <thead class="sticky top-0 z-10">
              <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)] backdrop-blur">
                <th class="px-3 py-2.5 font-medium">设备名</th>
                <th class="px-3 py-2.5 font-medium">版本</th>
                <th class="px-3 py-2.5 font-medium">提供商</th>
                <th class="px-3 py-2.5 font-medium">日期</th>
                <th class="px-3 py-2.5 font-medium">INF</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-[var(--card-border)]">
              <tr
                v-for="(d, i) in filteredDrivers"
                :key="`${d.inf_name}-${i}`"
                class="transition-colors hover:bg-[var(--card-soft)]"
              >
                <td class="max-w-[240px] truncate px-3 py-2.5 font-medium text-[var(--text-strong)]" :title="d.device_name">
                  {{ d.device_name || "—" }}
                </td>
                <td class="px-3 py-2.5 font-mono">{{ d.version || "—" }}</td>
                <td class="max-w-[180px] truncate px-3 py-2.5" :title="d.provider">{{ d.provider || "—" }}</td>
                <td class="px-3 py-2.5 tabular-nums">{{ d.date || "—" }}</td>
                <td class="max-w-[160px] truncate px-3 py-2.5 font-mono" :title="d.inf_name">{{ d.inf_name || "—" }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>
    </Card>
  </div>
</template>
