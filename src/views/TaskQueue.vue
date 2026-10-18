<script setup lang="ts">
/**
 * 任务队列页面。
 *
 * 除渲染队列外，本页还负责：
 * - 崩溃恢复：mounted 读 `load_queue_snapshot`，异常中断时弹恢复弹窗
 * - 快照写入：队列变化后节流 1.5s 调 `save_queue_snapshot`（恢复弹窗未决时不写，
 *   避免覆盖待恢复的快照）
 * - 队列选项（并发 / 限速 / 自动静默安装 / 完成关机 / 失败策略）
 * - 单任务安装参数（安装模式 + 自定义目录）
 * - 缓存目录入口与安装包清理
 *
 * 【需要父代理在 store / 后端补的能力 —— 页面均已做降级保护，缺失时不会报错】
 * 1) store：`restoreTasks(list: QueueTask[]): number`
 *    把崩溃恢复的任务批量并入队列（按 id 去重，status 一律重置为 pending）。
 *    缺失时页面用 `tasks.$patch((s) => s.tasks.push(...))` 兜底。
 * 2) store：`QueueTask` 增加可选字段
 *    `installSilent?: boolean`、`installDir?: string`、`stage?: string`
 *    - 前两个承载本页「安装参数」弹窗的结果，并需透传给后端 taskopts / software.rs
 *      （现在页面只在内存 `taskParams` 里维护，刷新即丢）
 *    - `stage` 有值时本页优先用它判校验徽章，否则退回解析 message 文本
 * 3) 后端命令：`clean_task_cache(ids: string[]) -> Result<usize, String>`
 *    按任务 id 删除 `缓存\downloads\{id}.exe`（页面已 try/catch）
 * 4) 后端命令（父代理需在退出流程里接）：
 *    `mark_clean_exit()` —— 窗口关闭 / 应用正常退出时调用，删除运行标记；
 *    `save_queue_snapshot(snapshot)` —— 本页已主动调用，父代理可再兜一层。
 * 5) 已存在、本页直接复用：`tasks.setFinishAction`（关机开关）、`set_task_opts`、
 *    `get_setting` / `set_setting`、`open_path`、`choose_dir`、`get_dirs`。
 * 6) settings.json 里三个键目前只有本页读写，需要父代理在安装流程里消费：
 *    `auto_silent_install`（"1"/"0"）、`finish_action_shutdown`（"1"/"0"）、
 *    `queue_fail_policy`（"stop"/"skip"）。
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  Play, Pause, Square, Trash2, FileText, X, GripVertical, RotateCcw, Timer, Sparkles,
  SlidersHorizontal, ChevronDown, ChevronUp, Settings2, FolderOpen, Eraser, HardDrive,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Progress from "../components/ui/Progress.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import Select from "../components/ui/Select.vue";
import Input from "../components/ui/Input.vue";
import Switch from "../components/ui/Switch.vue";
import Dialog from "../components/ui/Dialog.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { useRouteStore } from "../stores/route";
import {
  useTasksStore, FINISH_ACTIONS,
  type QueueTask, type TaskStatus, type TaskKind, type TaskFilter,
} from "../stores/tasks";
import { getDirs, chooseDir } from "../lib/tauri";
import { toast } from "../lib/toast";

const tasks = useTasksStore();
const route = useRouteStore();

/* ---- 后端契约（内联声明，避免依赖 lib/tauri 未导出的类型） ---- */
interface TaskSnapshotDTO {
  id: string;
  name: string;
  /** "pending" | "running" | "paused" | "done" | "failed" */
  status: string;
  progress: number;
  /** "download" | "verify" | "install" | "done" | "error" */
  stage: string;
  message: string;
  downloaded: number;
  total: number;
  file: string;
  code: string;
  updated: number;
}

interface QueueSnapshotDTO {
  tasks: TaskSnapshotDTO[];
  running: boolean;
  saved_at: number;
  interrupted: boolean;
}

interface TaskOptsDTO {
  mode: string;
  concurrency: number;
  speed_limit_kb: number;
}

const statusMeta: Record<TaskStatus, { label: string; tone: "primary" | "success" | "warning" | "danger" | "gray" }> = {
  pending: { label: "等待中", tone: "gray" },
  running: { label: "执行中", tone: "primary" },
  paused: { label: "已暂停", tone: "warning" },
  done: { label: "已完成", tone: "success" },
  failed: { label: "失败/终止", tone: "danger" },
};

const kindMeta: Record<TaskKind, { icon: string; label: string }> = {
  install: { icon: "📥", label: "安装" },
  uninstall: { icon: "🗑️", label: "卸载" },
  backup: { icon: "💾", label: "备份导出" },
  custom: { icon: "🔧", label: "自定义" },
};

const FILTER_OPTIONS: { value: TaskFilter; label: string }[] = [
  { value: "all", label: "全部任务" },
  { value: "install", label: "仅安装" },
  { value: "uninstall", label: "仅卸载" },
  { value: "backup", label: "仅备份任务" },
];

/* ================= 1. 崩溃恢复 ================= */

const recoveryOpen = ref(false);
const recoverySnapshot = ref<QueueSnapshotDTO | null>(null);
/** "restore" | "discard" */
const recoveryChoice = ref<"restore" | "discard">("restore");
/** 恢复弹窗未处理完前不写快照，避免把待恢复的数据覆盖掉 */
const snapshotLocked = ref(true);

/** 待恢复的任务（仅 pending / running 的，其余不复活） */
const recoveryTasks = computed(() =>
  (recoverySnapshot.value?.tasks ?? []).filter(
    (t) => t.status === "pending" || t.status === "running"
  )
);

async function checkRecovery() {
  // 队列里已经有任务（用户是从软件库加完再进来的）：不弹恢复窗，免得把两批混在一起
  if (tasks.tasks.length > 0) {
    snapshotLocked.value = false;
    return;
  }
  try {
    const snap = await invoke<QueueSnapshotDTO>("load_queue_snapshot");
    if (snap && snap.interrupted && snap.tasks.length > 0) {
      recoverySnapshot.value = snap;
      recoveryChoice.value = "restore";
      if (recoveryTasks.value.length > 0) {
        recoveryOpen.value = true;
        return;
      }
      // 上次中断但没有可恢复的任务（都已完成/失败）：不打扰用户
      recoverySnapshot.value = null;
    }
  } catch {
    // 后端命令未注册 / 读取失败：静默跳过，不影响队列本身
  }
  snapshotLocked.value = false;
}

function cancelRecovery() {
  // 用户关掉弹窗没做决定：快照文件原样保留（下次进页面还会再问）。
  // 这里解除写锁定 —— flushSnapshot 在空队列时直接 return，不会再把旧快照覆盖掉。
  recoveryOpen.value = false;
  snapshotLocked.value = false;
}

function onRecoveryOpen(v: boolean) {
  if (v) recoveryOpen.value = true;
  else cancelRecovery();
}

async function applyRecovery() {
  const snap = recoverySnapshot.value;
  if (!snap) return;
  recoveryOpen.value = false;

  if (recoveryChoice.value === "discard") {
    try {
      await invoke("clear_queue_snapshot");
    } catch {
      /* 命令缺失时忽略 */
    }
    recoverySnapshot.value = null;
    snapshotLocked.value = false;
    toast("已放弃未完成的任务", "info");
    return;
  }

  const list: QueueTask[] = recoveryTasks.value.map((t) => ({
    id: t.id,
    name: t.name,
    kind: "install",
    status: "pending",
    progress: 0,
    message: t.downloaded > 0 ? "等待继续下载" : "恢复后等待开始",
  }));

  const store = tasks as unknown as {
    restoreTasks?: (l: QueueTask[]) => number;
    $patch: (fn: (s: { tasks: QueueTask[] }) => void) => void;
  };
  if (typeof store.restoreTasks === "function") {
    store.restoreTasks(list);
  } else {
    // 兜底：store 还没接 restoreTasks 时直接 patch（父代理接入后可删掉此分支）
    const ids = new Set(tasks.tasks.map((t) => t.id));
    store.$patch((s) => {
      for (const item of list) {
        if (!ids.has(item.id)) s.tasks.push(item);
      }
    });
  }

  recoverySnapshot.value = null;
  snapshotLocked.value = false;
  toast(`已恢复 ${list.length} 个任务，点击「开始」继续`, "success");
}

/* ================= 2. 队列选项 ================= */

/** 限速档位 → KB/s（0 = 不限） */
const SPEED_OPTIONS: { value: string; label: string }[] = [
  { value: "0", label: "不限速" },
  { value: "1024", label: "1 MB/s" },
  { value: "2048", label: "2 MB/s" },
  { value: "5120", label: "5 MB/s" },
  { value: "10240", label: "10 MB/s" },
];

const CONCURRENCY_OPTIONS: { value: string; label: string }[] = [1, 2, 3, 4, 5, 6, 7, 8].map(
  (n) => ({ value: String(n), label: `${n} 个并发` })
);

const FAIL_POLICY_OPTIONS: { value: string; label: string }[] = [
  { value: "stop", label: "停止队列" },
  { value: "skip", label: "跳过继续" },
];

const optsOpen = ref(false);
const optConcurrency = ref("2");
const optSpeed = ref("0");
const optAutoSilent = ref(true);
const optFailPolicy = ref("stop");
/** 关机开关与 PageHeader 的「完成后动作」是同一个状态，直接读 store，避免两处打架 */
const optShutdown = computed(() => tasks.finishAction === "shutdown");

const optsSummary = computed(() => {
  const c = Number(optConcurrency.value) || 1;
  const sp = SPEED_OPTIONS.find((o) => o.value === optSpeed.value)?.label ?? "不限速";
  const bits = [`并发 ${c}`, sp];
  if (optAutoSilent.value) bits.push("完成后静默安装");
  if (optShutdown.value) bits.push("完成后关机");
  return `当前：${bits.join(" · ")}`;
});

async function loadOpts() {
  try {
    const o = await invoke<TaskOptsDTO>("get_task_opts");
    if (o) {
      optConcurrency.value = String(Math.min(8, Math.max(1, o.concurrency || 1)));
      const kb = String(o.speed_limit_kb || 0);
      optSpeed.value = SPEED_OPTIONS.some((x) => x.value === kb) ? kb : "0";
    }
  } catch {
    /* 命令缺失：保持默认 */
  }
  const [silent, policy] = await Promise.all([
    invoke<string>("get_setting", { key: "auto_silent_install" }).catch(() => ""),
    invoke<string>("get_setting", { key: "queue_fail_policy" }).catch(() => ""),
  ]);
  optAutoSilent.value = silent !== "0";
  if (policy === "skip" || policy === "stop") optFailPolicy.value = policy;
}

/** 并发 / 限速落到 settings.json 的 task_opts（后端参数为 snake_case） */
async function saveTaskOpts(concurrency: string, speed: string): Promise<boolean> {
  try {
    await invoke("set_task_opts", {
      // 后端只有 serial / parallel 两种模式，并发数 > 1 即并行
      mode: Number(concurrency) > 1 ? "parallel" : "serial",
      concurrency: Number(concurrency) || 1,
      speed_limit_kb: Number(speed) || 0,
    });
    return true;
  } catch (e) {
    // 后端 taskopts 目前把 concurrency 钳在 1~5；选 6~8 会被拒绝，这里明确提示
    toast(`保存并发/限速失败（后端并发上限可能仍为 5）：${String(e)}`, "error");
    return false;
  }
}

async function writeSetting(key: string, value: string) {
  await invoke("set_setting", { key, value }).catch(() => {});
}

async function onConcurrency(v: string) {
  const prev = optConcurrency.value;
  if (await saveTaskOpts(v, optSpeed.value)) {
    optConcurrency.value = v;
    toast(`并发数已设为 ${v}`, "success");
  } else {
    optConcurrency.value = prev;
  }
}
async function onSpeed(v: string) {
  const prev = optSpeed.value;
  if (await saveTaskOpts(optConcurrency.value, v)) {
    optSpeed.value = v;
    toast(`限速已设为：${SPEED_OPTIONS.find((o) => o.value === v)?.label}`, "success");
  } else {
    optSpeed.value = prev;
  }
}
async function onAutoSilent(v: boolean) {
  optAutoSilent.value = v;
  await writeSetting("auto_silent_install", v ? "1" : "0");
  toast(v ? "下载完成后将自动静默安装" : "已关闭自动静默安装", "info");
}
async function onShutdown(v: boolean) {
  // 与 PageHeader 的「完成后动作」共用 store 状态：走 setFinishAction 才能同步
  // 后端 set_task_complete_action（关机 / 休眠的实际执行者）
  await tasks.setFinishAction(v ? "shutdown" : "none");
  // 同时落一份独立开关，便于后端 / 其它页面按语义读取
  await writeSetting("finish_action_shutdown", v ? "1" : "0");
  toast(v ? "全部任务完成后将自动关机" : "已关闭自动关机", "info");
}
async function onFailPolicy(v: string) {
  optFailPolicy.value = v;
  await writeSetting("queue_fail_policy", v);
  toast(`失败策略：${FAIL_POLICY_OPTIONS.find((o) => o.value === v)?.label}`, "success");
}

/* ================= 3. 单任务安装参数 ================= */

interface TaskParams {
  silent: boolean;
  installDir: string;
}

/**
 * 任务 id → 安装参数。仅内存态：QueueTask 没有对应字段，
 * 需要父代理加 `installSilent?: boolean` / `installDir?: string` 并透传给后端 taskopts。
 */
const taskParams = ref<Record<string, TaskParams>>({});

const paramsOpen = ref(false);
const paramsTaskId = ref("");
const paramsTaskName = ref("");
const draftSilent = ref(true);
const draftDir = ref("");

function paramsOf(id: string): TaskParams {
  return taskParams.value[id] ?? { silent: true, installDir: "" };
}

function openParams(t: QueueTask) {
  const p = paramsOf(t.id);
  paramsTaskId.value = t.id;
  paramsTaskName.value = t.name;
  draftSilent.value = p.silent;
  draftDir.value = p.installDir;
  paramsOpen.value = true;
}

async function browseDir() {
  const dir = await chooseDir("选择安装目录");
  if (dir) draftDir.value = dir;
}

function saveParams() {
  taskParams.value = {
    ...taskParams.value,
    [paramsTaskId.value]: {
      silent: draftSilent.value,
      installDir: draftDir.value.trim(),
    },
  };
  paramsOpen.value = false;
  toast(`已保存「${paramsTaskName.value}」的安装参数`, "success");
}

/* ================= 4. 缓存区 ================= */

const cacheDir = ref("");
const showCleanCache = ref(false);

async function openCacheDir() {
  try {
    if (!cacheDir.value) {
      const dirs = await getDirs();
      cacheDir.value = dirs?.cache ?? "";
    }
    if (!cacheDir.value) {
      toast("未获取到缓存目录", "error");
      return;
    }
    await invoke("open_path", { path: cacheDir.value });
  } catch (e) {
    toast(`打开缓存目录失败：${String(e)}`, "error");
  }
}

/** 已完成（安装成功）的任务 id —— 其安装包可以安全清理 */
const cleanableIds = computed(() =>
  tasks.tasks.filter((t) => t.kind === "install" && t.status === "done").map((t) => t.id)
);

async function cleanTaskCache() {
  const ids = cleanableIds.value;
  if (ids.length === 0) {
    toast("没有已完成的安装任务需要清理", "info");
    return;
  }
  try {
    await invoke<number>("clean_task_cache", { ids });
    toast(`已清理 ${ids.length} 个任务的安装包`, "success");
  } catch (e) {
    // 命令可能尚未由父代理新增
    toast(`清理失败（clean_task_cache 可能未实现）：${String(e)}`, "error");
  }
}

/* ================= 5. 哈希校验徽章 ================= */

/**
 * 校验状态记录表（任务 id → 状态）。
 * store 的 QueueTask 目前没有 stage 字段，只能从 message 文本与整体进度推断；
 * 记录后保持住——校验阶段结束后 message 会被安装阶段覆盖，否则徽章会闪一下就消失。
 */
const verifyMap = ref<Record<string, "checking" | "ok" | "fail">>({});

function refreshVerify() {
  const next: Record<string, "checking" | "ok" | "fail"> = {};
  for (const t of tasks.tasks) {
    if (t.status === "pending") continue; // 重新入队：清掉上一次的校验结果
    // 若父代理将来给 QueueTask 补了 stage 字段，优先用它
    const stage = (t as QueueTask & { stage?: string }).stage;
    let s: "checking" | "ok" | "fail" | null = null;
    if (stage === "verify") {
      s = t.status === "failed" ? "fail" : t.progress >= 94 ? "ok" : "checking";
    } else if (t.message.includes("校验失败")) {
      s = "fail";
    } else if (t.message.includes("校验通过")) {
      s = "ok";
    } else if (t.message.includes("校验")) {
      s = "checking";
    }
    if (s) next[t.id] = s;
    else if (verifyMap.value[t.id]) next[t.id] = verifyMap.value[t.id];
  }
  // 内容无变化时不换引用，避免每次进度推送都触发一次整表重渲染
  const prev = verifyMap.value;
  const same =
    Object.keys(next).length === Object.keys(prev).length &&
    Object.entries(next).every(([k, v]) => prev[k] === v);
  if (!same) verifyMap.value = next;
}

/** 当前任务应显示的校验徽章；null = 不显示 */
function verifyState(t: QueueTask): "checking" | "ok" | "fail" | null {
  return verifyMap.value[t.id] ?? null;
}

/* ================= 快照写入（节流） ================= */

let saveTimer: number | null = null;

/** 从进度文案解析已下载 / 总字节（后端格式："已下载 12 MB / 总计 34 MB"） */
function parseBytes(msg: string): { downloaded: number; total: number } {
  const m = /已下载 (\d+(?:\.\d+)?) MB \/ 总计 (\d+(?:\.\d+)?) MB/.exec(msg);
  if (!m) return { downloaded: 0, total: 0 };
  return {
    downloaded: Math.round(Number(m[1]) * 1048576),
    total: Math.round(Number(m[2]) * 1048576),
  };
}

/** 推断快照阶段：store 有 stage 字段就用它，否则按状态 / 进度近似（store 进度刻度：下载 0-90、校验 90-94、安装 94-100） */
function stageOf(t: QueueTask): string {
  const real = (t as QueueTask & { stage?: string }).stage;
  if (real) return real;
  if (t.status === "done") return "done";
  if (t.status === "failed") return "error";
  if (verifyMap.value[t.id] === "checking") return "verify";
  if (t.progress >= 94) return "install";
  if (t.progress >= 90) return "verify";
  return "download";
}

function flushSnapshot() {
  if (snapshotLocked.value) return;
  // 空队列不写快照，也不要顺手删旧快照：
  // 用户可能刚关掉恢复弹窗还没决定，删掉就再也捡不回来了。
  if (tasks.tasks.length === 0) return;
  const snapshot = {
    tasks: tasks.tasks.map((t) => {
      const { downloaded, total } = parseBytes(t.message);
      return {
        id: t.id,
        name: t.name,
        status: t.status,
        progress: t.progress,
        stage: stageOf(t),
        message: t.message,
        downloaded,
        total,
        // 后端缓存命名规则：<cache>\downloads\<id>.exe
        file: t.kind === "install" && cacheDir.value
          ? `${cacheDir.value}\\downloads\\${t.id}.exe`
          : "",
        code: t.code ?? "",
        updated: Math.floor(Date.now() / 1000),
      };
    }),
    running: tasks.running,
    saved_at: 0,
    interrupted: false,
  };
  void invoke("save_queue_snapshot", { snapshot }).catch(() => {});
}

/**
 * 节流（不是防抖）：下载阶段进度事件非常密集，防抖会一直重置计时器导致永远不落盘。
 * 已有待执行任务时直接返回，flush 时读的是当时最新的队列状态。
 */
function scheduleSnapshot() {
  if (saveTimer !== null) return;
  saveTimer = window.setTimeout(() => {
    saveTimer = null;
    flushSnapshot();
  }, 1500);
}

watch(
  () => [tasks.tasks, tasks.running] as const,
  () => {
    refreshVerify();
    scheduleSnapshot();
  },
  { deep: true }
);

onMounted(async () => {
  refreshVerify();
  // 缓存目录仅用于底部提示，取不到就留空（点击按钮时还会再试一次）
  void getDirs().then((d) => {
    cacheDir.value = d?.cache ?? "";
  });
  // loadOpts / checkRecovery 并行：选项面板读设置，恢复弹窗读快照，互不依赖
  await Promise.all([loadOpts(), checkRecovery()]);
  // 队列恢复 / 载入后立刻写一次基线快照
  scheduleSnapshot();
});

onBeforeUnmount(() => {
  if (saveTimer !== null) window.clearTimeout(saveTimer);
  saveTimer = null;
  flushSnapshot();
});

/* ---- 终止全部：二选弹窗 ---- */
const showTerminate = ref(false);
function confirmTerminate(deleteCache: boolean) {
  showTerminate.value = false;
  void tasks.terminate(deleteCache);
  toast(deleteCache ? "已终止并删除全部缓存" : "已终止任务（缓存已保留）", "info");
}

/* ---- 清空队列确认 ---- */
const showClear = ref(false);
function confirmClear() {
  tasks.clear();
  // 用户主动清空 = 明确放弃，连快照一起删，避免下次启动弹恢复窗
  void invoke("clear_queue_snapshot").catch(() => {});
  toast("任务队列已清空", "info");
}

/* ---- 拖拽排序（HTML5 drag & drop） ---- */
const dragId = ref<string | null>(null);
const overId = ref<string | null>(null);
function onDragStart(id: string) {
  dragId.value = id;
}
function onDragOver(id: string) {
  overId.value = id;
}
function onDrop(id: string) {
  if (dragId.value && dragId.value !== id) tasks.move(dragId.value, id);
  dragId.value = null;
  overId.value = null;
}
function onDragEnd() {
  dragId.value = null;
  overId.value = null;
}

function viewLog() {
  route.go("logs");
}

/** 秒 → "1 分 20 秒" / "45 秒" */
function durText(ms: number) {
  if (!ms || ms < 0) return "";
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} 秒`;
  const m = Math.floor(s / 60);
  const rest = s % 60;
  if (m < 60) return rest ? `${m} 分 ${rest} 秒` : `${m} 分`;
  const h = Math.floor(m / 60);
  return `${h} 小时 ${m % 60} 分`;
}

function taskElapsed(t: { _startTs?: number; _endTs?: number }) {
  if (!t._startTs) return "";
  return durText((t._endTs ?? Date.now()) - t._startTs);
}

/** 重试单个任务，并自动开始 */
async function retryOne(id: string) {
  if (!tasks.retry(id)) return;
  toast("已重新加入队列", "info");
  await tasks.start();
}

/** 重试全部失败任务 */
async function retryAll() {
  const n = tasks.retryAllFailed();
  if (n === 0) return;
  toast(`已重试 ${n} 个失败任务`, "info");
  await tasks.start();
}

/** 只清掉已完成/失败的，保留进行中的 */
function clearFinished() {
  const n = tasks.clearFinished();
  toast(n > 0 ? `已清理 ${n} 个结束的任务` : "没有需要清理的任务", n > 0 ? "success" : "info");
}

const finishedCount = computed(() => tasks.doneCount + tasks.failedCount);
</script>

<template>
  <div>
    <PageHeader title="任务队列" desc="安装 / 卸载 / 备份任务统一排队执行">
      <template #extra>
        <div class="flex flex-wrap items-center gap-2">
          <span class="text-xs text-[var(--muted)]">完成后动作</span>
          <Select
            :model-value="tasks.finishAction"
            :options="FINISH_ACTIONS.map((a) => ({ value: a.id, label: a.label }))"
            class="w-32"
            @update:model-value="(v) => tasks.setFinishAction(v as never)"
          />
        </div>
      </template>
    </PageHeader>

    <!-- 队列选项面板（默认收起为一行摘要，点击展开/折叠） -->
    <Card class="mb-4 overflow-hidden">
      <button
        type="button"
        class="flex w-full flex-wrap items-center gap-2 px-4 py-3 text-left text-sm transition-colors hover:bg-[var(--card-soft)]"
        @click="optsOpen = !optsOpen"
      >
        <SlidersHorizontal class="h-4 w-4 shrink-0 text-[var(--primary)]" />
        <span class="shrink-0 font-medium text-[var(--text-strong)]">队列选项</span>
        <span class="min-w-0 flex-1 truncate text-xs text-[var(--muted)]">{{ optsSummary }}</span>
        <ChevronUp v-if="optsOpen" class="h-4 w-4 shrink-0 text-[var(--muted)]" />
        <ChevronDown v-else class="h-4 w-4 shrink-0 text-[var(--muted)]" />
      </button>

      <div v-if="optsOpen" class="border-t border-[var(--card-border)] px-4 py-4">
        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <div class="min-w-0">
            <p class="mb-1.5 text-xs text-[var(--muted)]">并发下载数</p>
            <Select
              :model-value="optConcurrency"
              :options="CONCURRENCY_OPTIONS"
              class="w-full"
              @update:model-value="onConcurrency"
            />
          </div>
          <div class="min-w-0">
            <p class="mb-1.5 text-xs text-[var(--muted)]">下载限速</p>
            <Select
              :model-value="optSpeed"
              :options="SPEED_OPTIONS"
              class="w-full"
              @update:model-value="onSpeed"
            />
          </div>
          <div class="min-w-0">
            <p class="mb-1.5 text-xs text-[var(--muted)]">失败策略</p>
            <Select
              :model-value="optFailPolicy"
              :options="FAIL_POLICY_OPTIONS"
              class="w-full"
              @update:model-value="onFailPolicy"
            />
          </div>
          <div class="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-[var(--card-soft)] px-3 py-2">
            <div class="min-w-0">
              <p class="truncate text-sm">下载完成后自动静默安装</p>
              <p class="truncate text-xs text-[var(--muted)]">关闭则下载完只入缓存，不自动执行</p>
            </div>
            <Switch :checked="optAutoSilent" @update:checked="onAutoSilent" />
          </div>
          <div class="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-[var(--card-soft)] px-3 py-2">
            <div class="min-w-0">
              <p class="truncate text-sm">全部完成后自动关机</p>
              <p class="truncate text-xs text-[var(--muted)]">60 秒缓冲，可 shutdown /a 取消</p>
            </div>
            <Switch :checked="optShutdown" @update:checked="onShutdown" />
          </div>
        </div>
        <p class="mt-3 truncate text-xs text-[var(--muted)]">{{ optsSummary }}</p>
      </div>
    </Card>

    <!-- 关机 / 休眠 警告提示 -->
    <div
      v-if="tasks.finishAction === 'shutdown' || tasks.finishAction === 'sleep'"
      class="mb-3 rounded-lg border border-[#f5a623]/40 bg-[#f5a623]/10 px-3 py-2 text-xs text-[#b7791f]"
    >
      ⚠ 已选择任务完成后自动{{ tasks.finishAction === "shutdown" ? "关机" : "休眠" }}：任务完成 60 秒后执行，可 shutdown /a 取消
    </div>

    <!-- 全局控制面板 -->
    <Card v-if="tasks.tasks.length > 0" class="mb-4 p-5">
      <div class="mb-2 flex flex-wrap items-center justify-between gap-2 text-sm">
        <span class="font-semibold">总体进度</span>
        <span class="text-[var(--muted)]">
          {{ tasks.statusText }} · {{ tasks.doneCount }}/{{ tasks.tasks.length }} 完成 · {{ tasks.totalProgress }}%
        </span>
      </div>
      <Progress
        :value="tasks.totalProgress"
        :tone="tasks.failedCount > 0 && !tasks.running ? 'danger' : tasks.totalProgress >= 100 ? 'success' : 'primary'"
        class="h-2.5"
      />
      <div v-if="tasks.totalElapsed > 0" class="mt-2 flex flex-wrap items-center gap-1 text-xs text-[var(--muted)]">
        <Timer class="h-3.5 w-3.5" /> 累计耗时 {{ durText(tasks.totalElapsed) }}
      </div>
      <div class="mt-4 flex flex-wrap gap-2">
        <Button v-if="!tasks.running && !tasks.paused" variant="primary" size="sm" :disabled="tasks.pendingCount === 0" @click="tasks.start()">
          <Play class="h-4 w-4" /> 开始
        </Button>
        <Button v-else-if="tasks.running && !tasks.paused" variant="gray" size="sm" @click="tasks.pause()">
          <Pause class="h-4 w-4" /> 暂停
        </Button>
        <Button v-else variant="primary" size="sm" @click="tasks.start()">
          <Play class="h-4 w-4" /> 继续
        </Button>
        <Button
          v-if="tasks.retryableCount > 0"
          variant="gray"
          size="sm"
          title="把所有失败的安装任务重新入队"
          @click="retryAll"
        >
          <RotateCcw class="h-4 w-4" /> 重试失败（{{ tasks.retryableCount }}）
        </Button>
        <Button variant="danger" size="sm" :disabled="!tasks.running && !tasks.paused && tasks.pendingCount === 0" @click="showTerminate = true">
          <Square class="h-4 w-4" /> 终止全部
        </Button>
      </div>
    </Card>

    <!-- 顶部筛选 -->
    <div v-if="tasks.tasks.length > 0" class="mb-3 flex flex-wrap items-center gap-2">
      <span class="text-xs text-[var(--muted)]">筛选</span>
      <Select
        :model-value="tasks.filter"
        :options="FILTER_OPTIONS"
        class="w-32"
        @update:model-value="(v) => (tasks.filter = v as TaskFilter)"
      />
    </div>

    <!-- 任务列表 -->
    <div v-if="tasks.tasks.length === 0" class="glass-card">
      <EmptyState icon="🗒️" text="队列为空" sub="在软件库勾选软件后点击「加入任务队列」" />
    </div>
    <div v-else class="space-y-3">
      <Card
        v-for="t in tasks.filteredTasks"
        :key="t.id"
        class="flex flex-wrap items-center gap-3 p-4 transition-opacity"
        :class="
          dragId === t.id
            ? 'opacity-40'
            : overId === t.id && dragId !== null && dragId !== t.id
              ? 'ring-2 ring-[var(--primary)]'
              : ''
        "
        @dragover.prevent="onDragOver(t.id)"
        @drop.prevent="onDrop(t.id)"
      >
        <!-- 拖拽把手 -->
        <span
          class="shrink-0 cursor-grab text-[var(--muted)] active:cursor-grabbing"
          title="拖拽调整顺序"
          draggable="true"
          @dragstart="onDragStart(t.id)"
          @dragend="onDragEnd"
        >
          <GripVertical class="h-4 w-4" />
        </span>
        <div
          class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl text-lg"
          :style="{ background: 'var(--primary-soft)', color: 'var(--primary)' }"
        >
          {{ kindMeta[t.kind].icon }}
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <span class="truncate font-medium">{{ t.name }}</span>
            <Badge tone="gray">{{ kindMeta[t.kind].label }}</Badge>
            <Badge :tone="statusMeta[t.status].tone">{{ statusMeta[t.status].label }}</Badge>
            <Badge v-if="verifyState(t) === 'checking'" tone="gray">校验中</Badge>
            <Badge v-else-if="verifyState(t) === 'ok'" tone="success">校验通过</Badge>
            <Badge v-else-if="verifyState(t) === 'fail'" tone="danger">校验失败</Badge>
            <Badge v-if="paramsOf(t.id).installDir" tone="primary">自定义目录</Badge>
            <Badge v-if="!paramsOf(t.id).silent" tone="warning">交互式安装</Badge>
          </div>
          <Progress
            :value="t.progress"
            class="mt-2"
            :tone="t.status === 'done' ? 'success' : t.status === 'failed' ? 'danger' : t.status === 'paused' ? 'warning' : 'primary'"
          />
          <p class="mt-1 truncate text-xs text-[var(--muted)]">
            {{ t.message }}<template v-if="t.speed"> · {{ t.speed }}</template><template v-if="t.eta"> · {{ t.eta }}</template><template v-if="t.code"> · 返回码/原因：{{ t.code }}</template><template v-if="taskElapsed(t)"> · 耗时 {{ taskElapsed(t) }}</template>
          </p>
        </div>
        <div class="flex shrink-0 flex-wrap items-center gap-1">
          <Button
            v-if="t.kind === 'install'"
            variant="ghost"
            size="icon"
            title="安装参数（静默 / 目录）"
            class="text-[var(--muted)] hover:text-[var(--primary)]"
            @click="openParams(t)"
          >
            <Settings2 class="h-4 w-4" />
          </Button>
          <Button v-if="t.status === 'failed'" variant="ghost" size="sm" class="text-[var(--primary)]" title="重新入队" @click="retryOne(t.id)">
            <RotateCcw class="h-4 w-4" /> 重试
          </Button>
          <Button variant="ghost" size="sm" @click="viewLog">
            <FileText class="h-4 w-4" /> 查看日志
          </Button>
          <Button
            v-if="t.status !== 'done'"
            variant="ghost"
            size="icon"
            title="从队列移除"
            class="text-[var(--muted)] hover:text-[#e5484d]"
            @click="tasks.remove(t.id)"
          >
            <X class="h-4 w-4" />
          </Button>
        </div>
      </Card>
    </div>

    <!-- 底部统计栏 -->
    <div v-if="tasks.tasks.length > 0" class="mt-4 flex flex-wrap items-center gap-4 px-1 text-xs text-[var(--muted)]">
      <span>共 {{ tasks.tasks.length }} 个任务</span>
      <span class="text-[#30a46c]">✔ 成功 {{ tasks.doneCount }}</span>
      <span class="text-[#e5484d]">✘ 失败 {{ tasks.failedCount }}</span>
      <span>⏳ 等待 {{ tasks.pendingCount }}</span>
      <span v-if="tasks.totalElapsed > 0">累计耗时 {{ durText(tasks.totalElapsed) }}</span>
      <div class="ml-auto flex flex-wrap items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          class="text-[var(--muted)]"
          :disabled="finishedCount === 0"
          title="只移除已完成和失败的任务"
          @click="clearFinished"
        >
          <Sparkles class="h-3.5 w-3.5" /> 清理已结束
        </Button>
        <Button variant="ghost" size="sm" class="text-[var(--muted)] hover:text-[#e5484d]" @click="showClear = true">
          <Trash2 class="h-3.5 w-3.5" /> 清空队列
        </Button>
      </div>
    </div>

    <!-- 缓存区 -->
    <div class="mt-4 flex flex-wrap items-center gap-3 rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] px-3 py-2 text-xs text-[var(--muted)]">
      <HardDrive class="h-3.5 w-3.5 shrink-0" />
      <span class="min-w-0 flex-1 truncate">
        安装包缓存：下载完成后保留在缓存目录，可复用于重装 / 离线安装{{ cacheDir ? `（${cacheDir}）` : "" }}
      </span>
      <div class="flex shrink-0 flex-wrap items-center gap-1">
        <Button variant="ghost" size="sm" class="text-[var(--muted)]" @click="openCacheDir">
          <FolderOpen class="h-3.5 w-3.5" /> 打开缓存目录
        </Button>
        <Button
          variant="ghost"
          size="sm"
          class="text-[var(--muted)]"
          :disabled="cleanableIds.length === 0"
          @click="showCleanCache = true"
        >
          <Eraser class="h-3.5 w-3.5" /> 清理已完成任务的安装包
        </Button>
      </div>
    </div>

    <!-- 终止全部：二选一 -->
    <Dialog :open="showTerminate" title="终止全部任务" @update:open="showTerminate = $event">
      <p class="text-sm leading-relaxed text-[var(--text)]">
        将终止所有等待中 / 执行中 / 已暂停的任务。已下载的安装包缓存如何处理？
      </p>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="showTerminate = false">取消</Button>
          <Button variant="gray" @click="confirmTerminate(false)">终止并保留缓存</Button>
          <Button variant="danger" @click="confirmTerminate(true)">终止并删除全部缓存</Button>
        </div>
      </template>
    </Dialog>

    <!-- 清理安装包确认 -->
    <ConfirmDialog
      :open="showCleanCache"
      title="清理安装包缓存"
      :message="`将删除 ${cleanableIds.length} 个已完成任务的安装包（缓存目录下的 exe）。已安装的软件不受影响，但下次重装需要重新下载。`"
      confirm-text="清理"
      danger
      @update:open="showCleanCache = $event"
      @confirm="cleanTaskCache"
    />

    <!-- 崩溃恢复弹窗 -->
    <Dialog :open="recoveryOpen" title="检测到上次未完成的任务" class="max-w-xl" @update:open="onRecoveryOpen">
      <div class="space-y-3">
        <p class="text-sm leading-relaxed text-[var(--text)]">
          上次程序没有正常退出（崩溃或被强制结束），队列中有
          <b class="text-[var(--primary)]">{{ recoveryTasks.length }}</b>
          个任务尚未完成。中断时正在下载的任务已保留已下载部分，恢复后可断点续传。
        </p>

        <div class="max-h-56 overflow-y-auto rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] p-2">
          <div
            v-for="t in recoveryTasks"
            :key="t.id"
            class="flex flex-wrap items-center gap-2 rounded-md px-2 py-1.5 text-xs"
          >
            <span class="min-w-0 flex-1 truncate">{{ t.name }}</span>
            <Badge tone="gray">{{ t.status === "running" ? "中断时执行中" : "等待中" }}</Badge>
            <span v-if="t.progress > 0" class="shrink-0 text-[var(--muted)]">{{ Math.round(t.progress) }}%</span>
          </div>
        </div>

        <div class="space-y-2">
          <label class="flex cursor-pointer flex-wrap items-center gap-2 rounded-lg px-1 py-1.5 hover:bg-[var(--card-soft)]">
            <input
              type="radio"
              name="recovery-choice"
              value="restore"
              :checked="recoveryChoice === 'restore'"
              class="h-4 w-4 shrink-0 accent-[var(--primary)]"
              @change="recoveryChoice = 'restore'"
            />
            <span class="text-sm">恢复全部任务</span>
            <span class="text-xs text-[var(--muted)]">重新加入队列，需手动点「开始」</span>
          </label>
          <label class="flex cursor-pointer flex-wrap items-center gap-2 rounded-lg px-1 py-1.5 hover:bg-[var(--card-soft)]">
            <input
              type="radio"
              name="recovery-choice"
              value="discard"
              :checked="recoveryChoice === 'discard'"
              class="h-4 w-4 shrink-0 accent-[var(--primary)]"
              @change="recoveryChoice = 'discard'"
            />
            <span class="text-sm">放弃所有未完成任务</span>
            <span class="text-xs text-[var(--muted)]">同时删除队列快照</span>
          </label>
        </div>
      </div>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="cancelRecovery">取消</Button>
          <Button variant="primary" @click="applyRecovery">
            {{ recoveryChoice === "discard" ? "放弃任务" : "恢复任务" }}
          </Button>
        </div>
      </template>
    </Dialog>

    <!-- 单任务安装参数 -->
    <Dialog :open="paramsOpen" :title="`安装参数 · ${paramsTaskName}`" @update:open="paramsOpen = $event">
      <div class="space-y-4">
        <div>
          <p class="mb-1.5 text-xs text-[var(--muted)]">安装模式</p>
          <div class="flex flex-wrap gap-2">
            <Button :variant="draftSilent ? 'primary' : 'gray'" size="sm" @click="draftSilent = true">
              静默安装
            </Button>
            <Button :variant="!draftSilent ? 'primary' : 'gray'" size="sm" @click="draftSilent = false">
              交互式安装
            </Button>
          </div>
          <p class="mt-1.5 text-xs text-[var(--muted)]">
            {{ draftSilent ? "使用安装包内置静默参数，全程无窗口" : "弹出安装向导，需要手动点击（会阻塞队列）" }}
          </p>
        </div>
        <div>
          <p class="mb-1.5 text-xs text-[var(--muted)]">自定义安装目录（留空使用安装包默认值）</p>
          <div class="flex flex-wrap items-center gap-2">
            <Input v-model="draftDir" placeholder="例如 D:\Program Files\App" class="min-w-0 flex-1" />
            <Button variant="gray" size="sm" @click="browseDir">
              <FolderOpen class="h-3.5 w-3.5" /> 浏览
            </Button>
          </div>
        </div>
        <p class="text-xs text-[var(--muted)]">
          注：安装参数目前仅保存在本页内存中，接入后端后会随任务一起下发。
        </p>
      </div>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="paramsOpen = false">取消</Button>
          <Button variant="primary" @click="saveParams">保存</Button>
        </div>
      </template>
    </Dialog>

    <!-- 清空队列确认 -->
    <ConfirmDialog
      :open="showClear"
      title="清空任务队列"
      message="确定要清空任务队列吗？执行中与等待中的任务都会被移除，该操作不可撤销。"
      confirm-text="清空"
      danger
      @update:open="showClear = $event"
      @confirm="confirmClear"
    />
  </div>
</template>
