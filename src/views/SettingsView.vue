<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Download, FolderOpen, Globe, Loader2, RefreshCw, Upload } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Dialog from "../components/ui/Dialog.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Switch from "../components/ui/Switch.vue";
import Spinner from "../components/ui/Spinner.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import CacheManagerDialog from "../components/CacheManagerDialog.vue";
import { ACCENTS, useThemeStore, type ThemeMode } from "../stores/theme";
import { useTasksStore } from "../stores/tasks";
import {
  cacheStats, cleanCache, exportConfig, getAutostart, getSetting, importConfig,
  setAutostart, setSetting, getTaskOpts, setTaskOpts,
  getAutoBackupInventory, setAutoBackupInventory, setLogRetention,
  relaunchRepair, checkUpdate,
  type CacheStats, type UpdateInfo,
  setCacheDir,
  instChooseDir,
  getDirs,
  chooseFile,
  openPath,
  testProxy,
  type ProxyTest,
  type Dirs,
} from "../lib/tauri";
import { formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

const theme = useThemeStore();
const tasks = useTasksStore();

// —— 常规
const autostart = ref<boolean | null>(null);
const stats = ref<CacheStats | null>(null);
const cleaning = ref(false);

// —— 任务默认选项（后端契约）
const taskMode = ref("serial");
const taskConcurrency = ref(2);
const taskSpeedLimit = ref("0");
const taskOptsReady = ref(false);

const taskModeOpts = [
  { value: "serial", label: "串行（推荐）" },
  { value: "parallel", label: "并行" },
];
const concurrencyOpts = [1, 2, 3, 4, 5].map((n) => ({
  value: String(n),
  label: `${n} 个`,
}));

// —— 缓存 / 日志自动清理周期
const cacheRetention = ref("0");
const logRetention = ref("0");
const retentionOpts = [
  { value: "0", label: "关闭" },
  { value: "7", label: "7 天" },
  { value: "15", label: "15 天" },
  { value: "30", label: "30 天" },
];

// —— 自动备份清单
const autoBackupInventory = ref<boolean | null>(null);

// —— 压测温度上限（超温自动终止）
const burnMaxTemp = ref("95");
const tempOpts = [
  { value: "75", label: "75℃（保守）" },
  { value: "85", label: "85℃" },
  { value: "90", label: "90℃" },
  { value: "95", label: "95℃（默认）" },
  { value: "100", label: "100℃" },
];

async function onTempChange(v: string) {
  burnMaxTemp.value = v;
  await persistSetting("burn_max_temp_c", v, `压测温度上限已设为 ${v}℃`);
}

// —— 缓存管理弹窗
const cacheDialogOpen = ref(false);

// —— 网络
const useProxy = ref(false);
const proxyHost = ref("127.0.0.1");
const proxyPort = ref("7890");

// —— 配置
const confirmReset = ref(false);
// —— 修复安装
const confirmRepair = ref(false);
const repairing = ref(false);

// —— 检查更新
const checkingUpdate = ref(false);
const updateInfo = ref<UpdateInfo | null>(null);
const updateDialogOpen = ref(false);

async function doCheckUpdate() {
  checkingUpdate.value = true;
  const res = await checkUpdate();
  checkingUpdate.value = false;
  if (!res) return; // 调用失败：tryCall 已弹出错误 toast
  if (!res.has_update) {
    toast("已是最新版本", "success");
    return;
  }
  updateInfo.value = res;
  updateDialogOpen.value = true;
}

/** 前往下载：window.open 在 Tauri WebView 中走系统默认浏览器 */
function openDownloadUrl() {
  const url = updateInfo.value?.url;
  if (url) window.open(url, "_blank", "noopener");
}

/* ---- 软件更新设置（独立于上方「关于」卡片的即点即查） ---- */
interface UpdateProbe {
  current: string;
  latest: string;
  /** 后端实际返回 has_update（snake_case）；兼容 camelCase 版本 */
  has_update?: boolean;
  hasUpdate?: boolean;
  url?: string;
  notes?: string;
}

const updAuto = ref(true);
const updPre = ref(false);
const updKeep = ref(true);
const updChecking = ref(false);
const updResult = ref("");

async function loadUpdatePrefs() {
  const [auto, pre, keep] = await Promise.all([
    invoke<string>("get_setting", { key: "auto_check_update" }).catch(() => ""),
    invoke<string>("get_setting", { key: "update_prerelease" }).catch(() => ""),
    invoke<string>("get_setting", { key: "update_keep_installer" }).catch(() => ""),
  ]);
  // 空串表示从未设置过：沿用界面默认值
  if (auto === "1" || auto === "0") updAuto.value = auto === "1";
  if (pre === "1" || pre === "0") updPre.value = pre === "1";
  if (keep === "1" || keep === "0") updKeep.value = keep === "1";
}

async function saveUpdatePref(key: string, value: string, on: boolean, label: string): Promise<boolean> {
  try {
    await invoke("set_setting", { key, value });
    toast(`${label}已${on ? "开启" : "关闭"}`, "success");
    return true;
  } catch (err) {
    toast(`保存失败：${String(err)}`, "error");
    return false;
  }
}

async function toggleUpdAuto(v: boolean) {
  updAuto.value = v;
  if (!(await saveUpdatePref("auto_check_update", v ? "1" : "0", v, "启动时自动检测更新"))) {
    updAuto.value = !v;
  }
}

async function toggleUpdPre(v: boolean) {
  updPre.value = v;
  if (!(await saveUpdatePref("update_prerelease", v ? "1" : "0", v, "预发布版本检测"))) {
    updPre.value = !v;
  }
}

async function toggleUpdKeep(v: boolean) {
  updKeep.value = v;
  if (!(await saveUpdatePref("update_keep_installer", v ? "1" : "0", v, "保留安装包"))) {
    updKeep.value = !v;
  }
}

/** 结果文字：当前版本 · 最新版本 · 结论 */
async function doCheckUpdateInline() {
  if (updChecking.value) return;
  updChecking.value = true;
  updResult.value = "";
  try {
    const r = await invoke<UpdateProbe>("check_update");
    if (!r) {
      updResult.value = "检查失败：没有拿到版本信息";
      return;
    }
    const cur = (r.current || "未知").replace(/^v/i, "");
    const latest = (r.latest || cur).replace(/^v/i, "");
    const has = r.has_update ?? r.hasUpdate ?? false;
    const tail = has ? "有新版本" : "已是最新版";
    const notes = (r.notes ?? "").trim();
    updResult.value = `当前 v${cur} · 最新 v${latest} · ${tail}${notes ? ` · ${notes}` : ""}`;
    toast(has ? `发现新版本 v${latest}` : "已是最新版", has ? "success" : "info");
  } catch (err) {
    updResult.value = `检查失败：${String(err)}`;
    toast("检查更新失败", "error");
  } finally {
    updChecking.value = false;
  }
}

async function doRelaunchRepair() {
  repairing.value = true;
  try {
    await relaunchRepair();
  } catch (err) {
    console.error("[tauri] relaunch_repair 调用失败:", err);
    repairing.value = false;
    toast("启动修复模式失败", "error");
  }
}

const modeOptions = [
  { value: "dark", label: "🌙 深色" },
  { value: "light", label: "☀️ 浅色" },
  { value: "system", label: "🖥 跟随系统" },
];

async function persistSetting(key: string, value: string, msg?: string) {
  const ok = await setSetting(key, value);
  if (ok !== null && msg) toast(msg, "success");
}

const cacheDir = ref("");
const changingCache = ref(false);
const dirsInfo = ref<Dirs | null>(null);

/** 数据目录一览：程序本体用户改不了，其余三项可定位 */
const DIR_ROWS = [
  { key: "program", label: "程序目录", hint: "本体所在位置" },
  { key: "config", label: "配置目录", hint: "settings.json 等" },
  { key: "backup", label: "备份导出", hint: "软件清单与配置备份" },
  { key: "cache", label: "缓存目录", hint: "下载临时文件与图标" },
  { key: "log", label: "日志目录", hint: "运行与操作日志" },
] as const;

async function openDir(key: string) {
  const p = (dirsInfo.value as unknown as Record<string, string> | null)?.[key];
  if (!p) {
    toast("目录信息尚未读取", "info");
    return;
  }
  try {
    await openPath(p);
  } catch (err) {
    toast(String(err), "error");
  }
}

async function changeCacheDir() {
  const d = await instChooseDir();
  if (!d) return;
  changingCache.value = true;
  const ok = await setCacheDir(d);
  changingCache.value = false;
  if (ok !== null) {
    cacheDir.value = d;
    toast("缓存目录已更新", "success");
  } else {
    toast("修改失败：目录非法或写入失败", "error");
  }
}

onMounted(async () => {
  const dirs = await getDirs();
  if (dirs) {
    dirsInfo.value = dirs;
    cacheDir.value = (dirs as unknown as Record<string, string>).cache || "";
  }
  void getAutostart().then((v) => (autostart.value = v ?? false));
  void cacheStats().then((s) => (stats.value = s));

  // 任务默认选项（后端契约，未接入时回退默认值）
  void getTaskOpts().then((o) => {
    if (o) {
      taskMode.value = o.mode === "parallel" ? "parallel" : "serial";
      taskConcurrency.value = Math.min(5, Math.max(1, o.concurrency));
      taskSpeedLimit.value = String(o.speed_limit_kb);
    }
    taskOptsReady.value = true;
  });

  // 自动备份清单开关（后端契约）
  void getAutoBackupInventory().then((v) => (autoBackupInventory.value = v ?? false));

  // 本地持久化设置读取（经后端 get_setting，失败回退 localStorage）
  const cr = await getSetting("cache_retention");
  if (cr) cacheRetention.value = cr;
  const lr = await getSetting("log_retention");
  if (lr) logRetention.value = lr;
  const up = await getSetting("use_system_proxy");
  if (up !== null) useProxy.value = up === "1";
  else useProxy.value = localStorage.getItem("jw-use-proxy") === "1";
  const ph = await getSetting("proxy_host");
  if (ph) proxyHost.value = ph;
  const pp = await getSetting("proxy_port");
  if (pp) proxyPort.value = pp;
  const bt = await getSetting("burn_max_temp_c");
  if (bt) burnMaxTemp.value = bt;

  // 软件更新设置三个开关
  void loadUpdatePrefs();
});

async function toggleAutostart(v: boolean) {
  const ok = await setAutostart(v);
  if (ok !== null) {
    autostart.value = v;
    toast(`开机自启已${v ? "开启" : "关闭"}`, "success");
  }
}

async function doClean() {
  cleaning.value = true;
  const ok = await cleanCache();
  cleaning.value = false;
  if (ok !== null) {
    toast("缓存清理完成", "success");
    void cacheStats().then((s) => (stats.value = s));
  }
}

/** 保存任务默认选项（模式/并发变更即时保存；限速输入失焦保存） */
async function saveTaskOpts() {
  if (!taskOptsReady.value) return;
  const limit = Math.max(0, Number.parseInt(taskSpeedLimit.value, 10) || 0);
  const ok = await setTaskOpts(taskMode.value, taskConcurrency.value, limit);
  if (ok !== null) toast("任务选项已保存", "success");
}

async function onTaskModeChange(v: string) {
  taskMode.value = v;
  await saveTaskOpts();
}

async function onConcurrencyChange(v: string) {
  taskConcurrency.value = Number(v);
  await saveTaskOpts();
}

function onSpeedLimitBlur() {
  taskSpeedLimit.value = String(Math.max(0, Number.parseInt(taskSpeedLimit.value, 10) || 0));
  void saveTaskOpts();
}

async function onCacheRetentionChange(v: string) {
  cacheRetention.value = v;
  await persistSetting(
    "cache_retention",
    v,
    `缓存自动清理已${v === "0" ? "关闭" : `设为 ${v} 天`}`
  );
}

async function onLogRetentionChange(v: string) {
  logRetention.value = v;
  const days = Number(v);
  const ok = await setLogRetention(days);
  if (ok !== null) {
    // 本地镜像，便于下次打开时回显
    await setSetting("log_retention", v);
    toast(`日志自动清理已${days === 0 ? "关闭" : `设为 ${days} 天`}`, "success");
  }
}

async function toggleAutoBackupInventory(v: boolean) {
  autoBackupInventory.value = v;
  const ok = await setAutoBackupInventory(v);
  if (ok !== null) {
    toast(`自动备份清单已${v ? "开启" : "关闭"}`, "success");
  } else {
    autoBackupInventory.value = !v;
  }
}

async function doExport() {
  const res = await exportConfig();
  if (res?.file) toast(`配置已导出到：${res.file}`, "success");
}

async function doImport() {
  // WebView2 里 window.prompt 常被禁用，改用系统文件选择框
  const file = await chooseFile("选择要导入的配置文件", "配置文件", "json");
  if (!file) return;
  const ok = await importConfig(file);
  if (ok !== null) toast("配置导入成功，部分设置可能需重启应用生效", "success");
}

function lsSet(key: string, v: string) {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* ignore */
  }
}

async function toggleUseProxy(v: boolean) {
  useProxy.value = v;
  await persistSetting("use_system_proxy", v ? "1" : "0", `系统代理已${v ? "开启" : "关闭"}`);
  lsSet("jw-use-proxy", v ? "1" : "0");
}

async function saveProxy() {
  await persistSetting("proxy_host", proxyHost.value);
  await persistSetting("proxy_port", proxyPort.value, "手动代理已保存");
}

/* ---- 代理连通性测试 ---- */
const proxyTesting = ref(false);
const proxyResult = ref<ProxyTest | null>(null);

async function doTestProxy() {
  if (proxyTesting.value) return;
  proxyTesting.value = true;
  proxyResult.value = null;
  try {
    proxyResult.value = await testProxy();
  } catch (err) {
    proxyResult.value = {
      ok: false,
      proxy: "",
      from_system: false,
      elapsed_ms: 0,
      message: String(err),
    };
  } finally {
    proxyTesting.value = false;
  }
}

/** 重置：主题回默认 + 本地偏好清空 */
async function doReset() {  theme.setMode("dark");
  theme.setAccent("blue");
  useProxy.value = false;
  proxyHost.value = "127.0.0.1";
  proxyPort.value = "7890";
  await persistSetting("use_system_proxy", "0");
  await persistSetting("proxy_host", proxyHost.value);
  await persistSetting("proxy_port", proxyPort.value);
  try {
    localStorage.removeItem("jw-theme-mode");
    localStorage.removeItem("jw-accent");
    localStorage.removeItem("jw-auto-backup");
    localStorage.removeItem("jw-use-proxy");
  } catch {
    /* ignore */
  }
  toast("已恢复默认设置", "success");
}
</script>

<template>
  <div>
    <PageHeader title="设置" desc="外观、常规、网络与配置备份" />

    <div class="grid grid-cols-1 gap-5 lg:grid-cols-2">
      <!-- ===== 常规 ===== -->
      <!-- 主题模式 -->
      <Card class="p-5">
        <h4 class="mb-1 font-semibold">界面主题</h4>
        <p class="mb-3 text-xs text-[var(--muted)]">深色 / 浅色 / 跟随系统</p>
        <div class="grid grid-cols-3 gap-2">
          <Button
            v-for="m in modeOptions"
            :key="m.value"
            :variant="theme.mode === m.value ? 'primary' : 'gray'"
            @click="theme.setMode(m.value as ThemeMode)"
          >
            {{ m.label }}
          </Button>
        </div>
      </Card>

      <!-- 主色调 -->
      <Card class="p-5">
        <h4 class="mb-1 font-semibold">主色调</h4>
        <p class="mb-3 text-xs text-[var(--muted)]">选择应用的主题色</p>
        <div class="flex gap-3">
          <button
            v-for="a in ACCENTS"
            :key="a.id"
            class="flex cursor-pointer flex-col items-center gap-1.5 rounded-xl border px-4 py-2.5 transition-all"
            :class="
              theme.accent === a.id
                ? 'border-[var(--primary)] bg-[var(--primary-soft)]'
                : 'border-[var(--card-border)] hover:border-[var(--primary)]'
            "
            @click="theme.setAccent(a.id)"
          >
            <span
              class="h-6 w-6 rounded-full shadow"
              :style="{ background: a.color, boxShadow: `0 0 10px ${a.color}66` }"
            />
            <span class="text-xs">{{ a.label }}</span>
          </button>
        </div>
      </Card>

      <!-- 开机自启 -->
      <Card class="flex items-center justify-between p-5">
        <div>
          <h4 class="font-semibold">开机自启动</h4>
          <p class="mt-1 text-xs text-[var(--muted)]">登录 Windows 时自动启动极维工具箱</p>
        </div>
        <Switch
          v-if="autostart !== null"
          :checked="autostart"
          @update:checked="toggleAutostart"
        />
        <Spinner v-else label="" />
      </Card>

      <!-- 缓存设置 -->
      <Card class="p-5">
        <div class="mb-3 flex items-center justify-between">
          <h4 class="font-semibold">缓存管理</h4>
            <div class="mb-2 w-full truncate text-xs text-[var(--muted)]" :title="cacheDir">当前缓存目录：{{ cacheDir || "读取中…" }}</div>
          <div class="flex gap-2">
            <Button variant="gray" size="sm" :disabled="changingCache" title="选择新的缓存目录（原缓存文件保留在原位置）" @click="changeCacheDir">
              {{ changingCache ? "修改中…" : "修改缓存目录" }}
            </Button>
            <Button variant="gray" size="sm" @click="cacheDialogOpen = true">
              打开缓存管理
            </Button>
            <Button
              variant="danger"
              size="sm"
              :disabled="cleaning || !stats || stats.total === 0"
              @click="doClean"
            >
              {{ cleaning ? "清理中…" : "一键清理" }}
            </Button>
          </div>
        </div>
        <div v-if="stats" class="flex items-baseline gap-2">
          <span class="text-2xl font-bold text-[var(--primary)]">{{ formatBytes(stats.total) }}</span>
          <span class="text-xs text-[var(--muted)]">共 {{ stats.dirs?.length ?? 0 }} 个缓存目录</span>
        </div>
        <Spinner v-else label="统计缓存占用…" />
        <div class="mt-3 flex items-center justify-between border-t border-[var(--card-border)] pt-3">
          <div>
            <p class="text-sm font-medium">缓存自动清理</p>
            <p class="mt-0.5 text-xs text-[var(--muted)]">超过设定周期的缓存文件启动时自动清除</p>
          </div>
          <Select
            :model-value="cacheRetention"
            :options="retentionOpts"
            class="w-28"
            @update:model-value="onCacheRetentionChange"
          />
        </div>
      </Card>

      <!-- 任务默认选项 -->
      <Card class="p-5">
        <h4 class="mb-1 font-semibold">任务默认选项</h4>
        <p class="mb-3 text-xs text-[var(--muted)]">任务队列全部完成后的动作</p>
        <div class="flex flex-wrap gap-2">
          <Button
            v-for="a in [{ id: 'none', label: '无' }, { id: 'notify', label: '通知' }, { id: 'log', label: '日志' }, { id: 'shutdown', label: '关机' }, { id: 'sleep', label: '休眠' }]"
            :key="a.id"
            :variant="tasks.finishAction === a.id ? 'primary' : 'gray'"
            size="sm"
            @click="tasks.setFinishAction(a.id as never)"
          >
            {{ a.label }}
          </Button>
        </div>
        <div class="mt-3 space-y-3 border-t border-[var(--card-border)] pt-3">
          <div class="flex items-center justify-between gap-3">
            <div>
              <p class="text-sm font-medium">下载模式</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">串行逐个下载更稳定，并行速度更快</p>
            </div>
            <Select
              :model-value="taskMode"
              :options="taskModeOpts"
              class="w-36"
              @update:model-value="onTaskModeChange"
            />
          </div>
          <div class="flex items-center justify-between gap-3">
            <div>
              <p class="text-sm font-medium">下载并发数</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">并行模式下的同时下载数量（1~5）</p>
            </div>
            <Select
              :model-value="String(taskConcurrency)"
              :options="concurrencyOpts"
              class="w-36"
              @update:model-value="onConcurrencyChange"
            />
          </div>
          <div class="flex items-center justify-between gap-3">
            <div>
              <p class="text-sm font-medium">下载限速</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">KB/s，0 或留空为不限速</p>
            </div>
            <Input
              v-model="taskSpeedLimit"
              type="number"
              placeholder="0"
              class="w-36 text-right"
              @blur="onSpeedLimitBlur"
            />
          </div>
        </div>
      </Card>

      <!-- 自动备份清单 -->
      <Card class="flex items-center justify-between p-5">
        <div>
          <h4 class="font-semibold">自动备份清单</h4>
          <p class="mt-1 text-xs text-[var(--muted)]">
            启动时自动生成本机软件清单简易备份，仅记录清单不保存安装包
          </p>
        </div>
        <Switch
          v-if="autoBackupInventory !== null"
          :checked="autoBackupInventory"
          @update:checked="toggleAutoBackupInventory"
        />
        <Spinner v-else label="" />
      </Card>

      <!-- 日志自动清理 -->
      <Card class="flex items-center justify-between p-5">
        <div>
          <h4 class="font-semibold">日志自动清理</h4>
          <p class="mt-1 text-xs text-[var(--muted)]">超过设定周期的运行日志自动删除，关闭则永久保留</p>
        </div>
        <Select
          :model-value="logRetention"
          :options="retentionOpts"
          class="w-28"
          @update:model-value="onLogRetentionChange"
        />
      </Card>

      <!-- 压测温度保护 -->
      <Card class="flex items-center justify-between p-5">
        <div class="min-w-0">
          <h4 class="font-semibold">压测温度保护</h4>
          <p class="mt-1 text-xs text-[var(--muted)]">
            CPU 烤机时温度达到该上限立即自动终止，防止硬件过热。设置过低会频繁中断压测。
          </p>
        </div>
        <Select
          :model-value="burnMaxTemp"
          :options="tempOpts"
          class="w-36 shrink-0"
          @update:model-value="onTempChange"
        />
      </Card>

      <!-- ===== 网络 ===== -->
      <Card class="p-5 lg:col-span-2">
        <div class="mb-1 flex items-center gap-2">
          <Globe class="h-4 w-4 text-[var(--primary)]" />
          <h4 class="font-semibold">网络</h4>
        </div>
        <div class="space-y-4">
          <div class="flex items-center justify-between">
            <div>
              <p class="text-sm font-medium">使用系统代理</p>
              <p class="text-xs text-[var(--muted)]">下载软件时跟随 Windows 系统代理设置</p>
            </div>
            <Switch :checked="useProxy" @update:checked="toggleUseProxy" />
          </div>
          <div class="flex flex-wrap items-center gap-3">
            <p class="w-24 shrink-0 text-sm font-medium">手动代理</p>
            <Input v-model="proxyHost" placeholder="代理地址" class="max-w-[220px]" />
            <span class="text-[var(--muted)]">:</span>
            <Input v-model="proxyPort" placeholder="端口" class="max-w-[100px]" />
            <Button variant="gray" size="sm" @click="saveProxy">保存</Button>
            <Button variant="gray" size="sm" :disabled="proxyTesting" @click="doTestProxy">
              <Loader2 v-if="proxyTesting" class="h-3.5 w-3.5 animate-spin" />
              测试连通性
            </Button>
          </div>
          <p v-if="proxyResult" class="text-xs" :class="proxyResult.ok ? 'text-[#30a46c]' : 'text-[#e5484d]'">
            {{ proxyResult.ok ? "✓" : "✗" }} {{ proxyResult.message }}
            <span v-if="proxyResult.proxy" class="ml-1 font-mono text-[var(--muted)]">（{{ proxyResult.proxy }}）</span>
          </p>
          <p class="text-[10px] text-[var(--muted)]">
            代理设置对软件下载、运行库下载、WebView2 下载与更新检查全部生效；填写后请点保存。
          </p>
        </div>
      </Card>

      <!-- ===== 数据目录 ===== -->
      <Card class="p-5 lg:col-span-2">
        <h4 class="mb-1 font-semibold">数据目录</h4>
        <p class="mb-3 text-xs text-[var(--muted)]">本体、配置、备份、缓存与日志各自独立，切换缓存目录不会搬动已有文件</p>
        <div class="space-y-1">
          <div
            v-for="row in DIR_ROWS"
            :key="row.key"
            class="flex flex-wrap items-center gap-2 border-b border-[var(--card-border)] py-2 last:border-0"
          >
            <span class="w-20 shrink-0 text-xs text-[var(--muted)]">{{ row.label }}</span>
            <span
              class="min-w-0 flex-1 truncate font-mono text-xs text-[var(--text)]"
              :title="(dirsInfo as unknown as Record<string, string>)?.[row.key] || '读取中…'"
            >
              {{ (dirsInfo as unknown as Record<string, string>)?.[row.key] || "读取中…" }}
            </span>
            <span class="hidden shrink-0 text-[10px] text-[var(--muted)] sm:inline">{{ row.hint }}</span>
            <Button
              variant="gray"
              size="sm"
              :disabled="!dirsInfo"
              title="在资源管理器中打开"
              @click="openDir(row.key)"
            >
              <FolderOpen class="h-3 w-3" />
            </Button>
          </div>
        </div>
      </Card>

      <!-- ===== 配置备份与恢复 ===== -->
      <Card class="p-5 lg:col-span-2">
        <h4 class="mb-1 font-semibold">配置备份与恢复</h4>
        <p class="mb-3 text-xs text-[var(--muted)]">导出全部配置以便迁移，或导入已有配置</p>
        <div class="flex flex-wrap gap-3">
          <Button variant="gray" @click="doExport">
            <Download class="h-4 w-4" /> 导出全部配置
          </Button>
          <Button variant="gray" @click="doImport">
            <Upload class="h-4 w-4" /> 导入配置
          </Button>
          <Button variant="danger" @click="confirmReset = true">重置设置</Button>
        </div>
      </Card>

      <!-- ===== 修复安装 ===== -->
      <Card class="p-5 lg:col-span-2">
        <h4 class="mb-1 font-semibold">修复安装</h4>
        <p class="mb-3 text-xs text-[var(--muted)]">
          校验程序文件、重建目录配置、修复快捷方式与文件关联，不影响用户数据
        </p>
        <Button variant="gray" :disabled="repairing" @click="confirmRepair = true">修复安装</Button>
      </Card>

      <!-- ===== 软件更新设置 ===== -->
      <Card class="p-5 lg:col-span-2">
        <div class="mb-3 flex flex-wrap items-center gap-2">
          <RefreshCw class="h-4 w-4 shrink-0 text-[var(--primary)]" />
          <div class="min-w-0">
            <h4 class="font-semibold">软件更新设置</h4>
            <p class="text-xs text-[var(--muted)]">控制启动时的更新检测行为，以及更新成功后安装包的处理方式</p>
          </div>
        </div>

        <div class="space-y-3">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div class="min-w-0">
              <p class="text-sm font-medium">软件启动时自动检测更新</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">启动后静默查询一次版本信息，有新版本时提示</p>
            </div>
            <Switch :checked="updAuto" @update:checked="toggleUpdAuto" />
          </div>

          <div class="flex flex-wrap items-center justify-between gap-3">
            <div class="min-w-0">
              <p class="text-sm font-medium">预发布版本检测</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">包含 beta / preview 等尚未正式发布的版本，可能不够稳定</p>
            </div>
            <Switch :checked="updPre" @update:checked="toggleUpdPre" />
          </div>

          <div class="flex flex-wrap items-center justify-between gap-3">
            <div class="min-w-0">
              <p class="text-sm font-medium">更新成功后保留安装包</p>
              <p class="mt-0.5 text-xs text-[var(--muted)]">保留更新包便于回滚或批量分发，会额外占用磁盘空间</p>
            </div>
            <Switch :checked="updKeep" @update:checked="toggleUpdKeep" />
          </div>
        </div>

        <div class="mt-3 flex flex-wrap items-center gap-3 border-t border-[var(--card-border)] pt-3">
          <Button variant="gray" size="sm" :disabled="updChecking" @click="doCheckUpdateInline">
            <Loader2 v-if="updChecking" class="h-3.5 w-3.5 animate-spin" />
            <RefreshCw v-else class="h-3.5 w-3.5" />
            {{ updChecking ? "检查中…" : "立即检查更新" }}
          </Button>
          <p
            v-if="updResult"
            class="min-w-0 flex-1 break-all text-xs leading-relaxed text-[var(--text)]"
          >
            {{ updResult }}
          </p>
          <p v-else class="text-xs text-[var(--muted)]">点「立即检查更新」查询远程最新版本</p>
        </div>
      </Card>

      <!-- ===== 关于 ===== -->
      <Card class="p-5 lg:col-span-2">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h4 class="mb-1 font-semibold">关于极维工具箱</h4>
            <p class="text-xs text-[var(--muted)]">检查新版本并获取更新说明</p>
          </div>
          <Button variant="gray" :disabled="checkingUpdate" @click="doCheckUpdate">
            <RefreshCw class="h-4 w-4" :class="checkingUpdate && 'animate-spin'" />
            {{ checkingUpdate ? "检查中…" : "检查更新" }}
          </Button>
        </div>
      </Card>
    </div>

    <ConfirmDialog
      :open="confirmReset"
      danger
      title="重置设置"
      message="将恢复主题与偏好为默认值（不影响软件清单与备份数据）。确定继续吗？"
      confirm-text="重置"
      @confirm="doReset"
      @update:open="confirmReset = $event"
    />

    <ConfirmDialog
      :open="confirmRepair"
      title="修复安装"
      message="将重启程序进入修复模式，修复快捷方式、目录配置与文件关联（不影响用户数据）。确定继续吗？"
      confirm-text="重启并修复"
      @confirm="doRelaunchRepair"
      @update:open="confirmRepair = $event"
    />

    <CacheManagerDialog v-model:open="cacheDialogOpen" />

    <!-- 更新可用弹窗 -->
    <Dialog :open="updateDialogOpen" title="发现新版本" class="!max-w-md" @update:open="updateDialogOpen = $event">
      <div v-if="updateInfo" class="space-y-3">
        <div class="flex items-center gap-3 text-sm">
          <span class="rounded-full bg-[var(--btn-gray)] px-2.5 py-1 text-xs text-[var(--muted)]">
            当前 v{{ updateInfo.current || "未知" }}
          </span>
          <span class="text-[var(--muted)]">→</span>
          <span class="rounded-full bg-[var(--primary-soft)] px-2.5 py-1 text-xs font-semibold text-[var(--primary)]">
            最新版 v{{ updateInfo.latest }}
          </span>
        </div>
        <div
          v-if="updateInfo.notes"
          class="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3 text-xs leading-relaxed"
        >
          {{ updateInfo.notes }}
        </div>
        <p v-else class="text-xs text-[var(--muted)]">暂无更新说明</p>
      </div>
      <template #footer>
        <div class="flex justify-end gap-3">
          <Button variant="gray" @click="updateDialogOpen = false">稍后再说</Button>
          <Button variant="primary" :disabled="!updateInfo?.url" @click="openDownloadUrl">
            前往下载
          </Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>
