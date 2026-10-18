<script setup lang="ts">
// 定时自动备份：任务配置全部走内联 invoke，避免依赖尚未接线的 tauri.ts 封装。
import { computed, onMounted, ref } from "vue";
import {
  CalendarClock, Clock, Eraser, Pencil, Play, Plus, Timer, Trash2,
} from "lucide-vue-next";
import { invoke } from "@tauri-apps/api/core";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Badge from "../components/ui/Badge.vue";
import Switch from "../components/ui/Switch.vue";
import Dialog from "../components/ui/Dialog.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { toast } from "../lib/toast";

interface BackupTask {
  id: string;
  name: string;
  /** "daily" | "weekly" | "monthly" */
  period: string;
  /** "HH:MM" */
  time: string;
  /** weekly: 1-7（1=周一）；monthly: 1-28 */
  day: number;
  apps: string[];
  enabled: boolean;
  keep: number;
  last_run: number;
  last_result: string;
}

interface NextItem {
  id: string;
  name: string;
  next_ts: number;
  next_text: string;
}

interface ScheduleStatus {
  running: boolean;
  next: NextItem[];
}

/* ---------------- 常量 ---------------- */

const PERIOD_OPTS = [
  { value: "daily", label: "每日" },
  { value: "weekly", label: "每周" },
  { value: "monthly", label: "每月" },
];

const HOUR_OPTS = Array.from({ length: 24 }, (_, i) => ({
  value: String(i),
  label: `${String(i).padStart(2, "0")} 时`,
}));

const MINUTE_BASE = [0, 15, 30, 45];
const WEEK_OPTS = [
  { value: "1", label: "周一" },
  { value: "2", label: "周二" },
  { value: "3", label: "周三" },
  { value: "4", label: "周四" },
  { value: "5", label: "周五" },
  { value: "6", label: "周六" },
  { value: "7", label: "周日" },
];
const MONTH_DAY_OPTS = Array.from({ length: 28 }, (_, i) => ({
  value: String(i + 1),
  label: `${i + 1} 日`,
}));
const KEEP_OPTS = [3, 5, 10, 20].map((n) => ({ value: String(n), label: `保留 ${n} 份` }));

/* ---------------- 列表状态 ---------------- */

const tasks = ref<BackupTask[]>([]);
const status = ref<ScheduleStatus>({ running: false, next: [] });
const loading = ref(true);
const busyId = ref("");

const nextMap = computed(() => {
  const m = new Map<string, NextItem>();
  for (const n of status.value.next) m.set(n.id, n);
  return m;
});

function periodText(t: BackupTask): string {
  if (t.period === "weekly") {
    const w = WEEK_OPTS.find((o) => Number(o.value) === t.day)?.label ?? `周${t.day}`;
    return `每${w} ${t.time}`;
  }
  if (t.period === "monthly") return `每月 ${t.day} 日 ${t.time}`;
  return `每日 ${t.time}`;
}

function resultTone(t: BackupTask): "success" | "danger" | "gray" {
  if (t.last_result.startsWith("成功")) return "success";
  if (t.last_result.startsWith("失败")) return "danger";
  return "gray";
}

function resultText(t: BackupTask): string {
  return t.last_result || "从未执行";
}

function lastRunText(t: BackupTask): string {
  if (!t.last_run) return "—";
  const d = new Date(t.last_run * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function nextText(t: BackupTask): string {
  if (!t.enabled) return "已停用";
  return nextMap.value.get(t.id)?.next_text || "—";
}

async function refresh() {
  const list = await invoke<BackupTask[]>("list_schedules").catch(() => null);
  if (list) tasks.value = list;
  const st = await invoke<ScheduleStatus>("get_schedule_status").catch(() => null);
  if (st) status.value = st;
}

async function load() {
  loading.value = true;
  await refresh();
  loading.value = false;
}

onMounted(load);

/* ---------------- 启用 / 删除 / 立即执行 ---------------- */

async function onToggle(t: BackupTask, v: boolean) {
  const prev = t.enabled;
  t.enabled = v;
  if (!v) {
    // 停用后不再展示"下次执行"
    status.value = { ...status.value, next: status.value.next.filter((n) => n.id !== t.id) };
  } else {
    status.value = { ...status.value, next: [...status.value.next, { id: t.id, name: t.name, next_ts: 0, next_text: "" }] };
  }
  try {
    tasks.value = await invoke<BackupTask[]>("toggle_schedule", { id: t.id, enabled: v });
    await refresh();
  } catch (e) {
    t.enabled = prev;
    toast(`切换失败：${e}`, "error");
    await refresh();
  }
}

const pendingDelete = ref<BackupTask | null>(null);

async function doDelete() {
  const target = pendingDelete.value;
  pendingDelete.value = null;
  if (!target) return;
  try {
    tasks.value = await invoke<BackupTask[]>("delete_schedule", { id: target.id });
    await refresh();
    toast(`已删除「${target.name}」`, "info");
  } catch (e) {
    toast(`删除失败：${e}`, "error");
  }
}

async function runNow(t: BackupTask) {
  if (busyId.value) return;
  busyId.value = t.id;
  try {
    const res = await invoke<{ file: string; time: number }>("run_schedule_now", { id: t.id });
    toast(`备份完成：${res.file.split(/[\\/]/).pop() ?? "已生成"}`, "success");
    await refresh();
  } catch (e) {
    toast(`备份失败：${e}`, "error");
    await refresh();
  } finally {
    busyId.value = "";
  }
}

/* ---------------- 新建 / 编辑弹窗 ---------------- */

const editorOpen = ref(false);
const saving = ref(false);
const form = ref<BackupTask>(blank());

function blank(): BackupTask {
  return {
    id: "",
    name: "",
    period: "daily",
    time: "03:00",
    day: 1,
    apps: [],
    enabled: true,
    keep: 5,
    last_run: 0,
    last_result: "",
  };
}

const hour = computed({
  get: () => String(Number(form.value.time.split(":")[0] ?? 3)),
  set: (v: string) => {
    form.value.time = `${v.padStart(2, "0")}:${minutePart.value}`;
  },
});

const minutePart = computed(() => {
  const m = form.value.time.split(":")[1] ?? "00";
  return m.padStart(2, "0");
});

const minute = computed({
  get: () => String(Number(minutePart.value)),
  set: (v: string) => {
    form.value.time = `${hour.value.padStart(2, "0")}:${v.padStart(2, "0")}`;
  },
});

/** 已有任务的分钟可能不在 0/15/30/45 上，动态补进选项，避免显示错位 */
const minuteOpts = computed(() => {
  const cur = Number(minutePart.value);
  const vals = MINUTE_BASE.includes(cur) ? MINUTE_BASE : [...MINUTE_BASE, cur].sort((a, b) => a - b);
  return vals.map((m) => ({ value: String(m), label: `${String(m).padStart(2, "0")} 分` }));
});

const weekDay = computed({
  get: () => String(form.value.day),
  set: (v: string) => {
    form.value.day = Number(v) || 1;
  },
});

const monthDay = computed({
  get: () => String(form.value.day),
  set: (v: string) => {
    form.value.day = Number(v) || 1;
  },
});

const keepValue = computed({
  get: () => String(form.value.keep),
  set: (v: string) => {
    form.value.keep = Number(v) || 5;
  },
});

function openCreate() {
  form.value = blank();
  editorOpen.value = true;
}

function openEdit(t: BackupTask) {
  form.value = { ...t, apps: [...t.apps] };
  editorOpen.value = true;
}

function onPeriodChange(v: string) {
  form.value.period = v;
  if (v === "weekly" && (form.value.day < 1 || form.value.day > 7)) form.value.day = 1;
  if (v === "monthly" && (form.value.day < 1 || form.value.day > 28)) form.value.day = 1;
  if (v === "daily") form.value.day = 1;
}

async function save() {
  const name = form.value.name.trim();
  if (!name) {
    toast("请填写任务名称", "warning");
    return;
  }
  saving.value = true;
  try {
    const payload: BackupTask = {
      ...form.value,
      name,
      time: `${hour.value.padStart(2, "0")}:${minutePart.value}`,
    };
    tasks.value = await invoke<BackupTask[]>("save_schedule", { task: payload });
    editorOpen.value = false;
    await refresh();
    toast("任务已保存", "success");
  } catch (e) {
    toast(`保存失败：${e}`, "error");
  } finally {
    saving.value = false;
  }
}

/* ---------------- 清理旧备份 ---------------- */

const cleanupKeep = ref("5");
const cleaning = ref(false);
const cleanupOpen = ref(false);

async function doCleanup() {
  cleaning.value = true;
  try {
    const n = await invoke<number>("cleanup_old_backups", { keep: Number(cleanupKeep.value) });
    toast(n > 0 ? `已删除 ${n} 份旧备份` : "没有需要清理的旧备份", n > 0 ? "success" : "info");
  } catch (e) {
    toast(`清理失败：${e}`, "error");
  } finally {
    cleaning.value = false;
  }
}
</script>

<template>
  <div>
    <PageHeader title="定时备份" desc="按日 / 周 / 月自动打包配置，备份文件存放在「备份导出」目录。">
      <template #extra>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="gray" size="sm" :disabled="cleaning" @click="cleanupOpen = true">
            <Eraser class="h-3.5 w-3.5" /> 清理旧备份
          </Button>
          <Button variant="primary" size="sm" @click="openCreate">
            <Plus class="h-3.5 w-3.5" /> 新建任务
          </Button>
        </div>
      </template>
    </PageHeader>

    <!-- 说明卡 -->
    <Card class="mb-4 p-5">
      <div class="flex flex-wrap items-start gap-3">
        <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-[var(--primary-soft)]">
          <CalendarClock class="h-4.5 w-4.5 text-[var(--primary)]" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <h3 class="font-semibold">自动备份说明</h3>
            <Badge :tone="status.running ? 'success' : 'gray'">
              {{ status.running ? "调度运行中" : "调度未启动" }}
            </Badge>
          </div>
          <p class="mt-1 text-xs leading-relaxed text-[var(--muted)]">
            定时备份只在工具箱运行时触发；软件完全关闭（包括退出托盘）不会执行，错过的任务会在下次启动后的检查中补跑一次。
          </p>
          <p class="mt-1 text-xs leading-relaxed text-[var(--muted)]">
            保留策略：每次执行成功后只保留最近 N 份备份，更早的会自动删除，避免备份目录无限增长。
          </p>
        </div>
      </div>
    </Card>

    <Spinner v-if="loading && tasks.length === 0" label="加载定时任务…" />

    <template v-else>
      <EmptyState
        v-if="tasks.length === 0"
        icon="⏰"
        text="还没有定时备份任务"
        sub="新建一个任务，让工具箱在指定时间自动打包配置；也可以随时手动执行一次。"
      />

      <div v-else class="space-y-3">
        <Card v-for="t in tasks" :key="t.id" class="p-5">
          <div class="flex flex-wrap items-start gap-3">
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="min-w-0 truncate font-semibold" :title="t.name">{{ t.name }}</span>
                <Badge tone="primary">{{ periodText(t) }}</Badge>
                <Badge v-if="!t.enabled" tone="gray">已停用</Badge>
              </div>

              <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs">
                <span class="flex items-center gap-1 text-[var(--muted)]">
                  <Clock class="h-3.5 w-3.5 shrink-0" />
                  下次执行
                  <b class="font-medium text-[var(--text)]">{{ nextText(t) }}</b>
                </span>
                <span class="flex items-center gap-1 text-[var(--muted)]">
                  <Timer class="h-3.5 w-3.5 shrink-0" />
                  上次 {{ lastRunText(t) }}
                </span>
                <span class="flex items-center gap-1 text-[var(--muted)]">
                  <Eraser class="h-3.5 w-3.5 shrink-0" />
                  保留 {{ t.keep }} 份
                </span>
              </div>

              <div class="mt-2 flex flex-wrap items-center gap-1.5">
                <span class="text-xs text-[var(--muted)]">最近结果</span>
                <Badge :tone="resultTone(t)" class="max-w-full">
                  <span class="min-w-0 truncate" :title="resultText(t)">{{ resultText(t) }}</span>
                </Badge>
              </div>
            </div>

            <div class="flex flex-wrap items-center gap-2">
              <Switch :checked="t.enabled" @update:checked="(v) => onToggle(t, !!v)" />
              <Button
                variant="primary"
                size="sm"
                :disabled="busyId === t.id"
                @click="runNow(t)"
              >
                <Play class="h-3.5 w-3.5" />
                {{ busyId === t.id ? "执行中…" : "立即执行" }}
              </Button>
              <Button variant="gray" size="sm" @click="openEdit(t)">
                <Pencil class="h-3.5 w-3.5" /> 编辑
              </Button>
              <Button
                variant="ghost"
                size="icon"
                title="删除任务"
                class="text-[var(--muted)] hover:text-[#e5484d]"
                @click="pendingDelete = t"
              >
                <Trash2 class="h-4 w-4" />
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </template>

    <!-- 新建 / 编辑 -->
    <Dialog
      :open="editorOpen"
      :title="form.id ? '编辑定时任务' : '新建定时任务'"
      @update:open="editorOpen = $event"
    >
      <div class="space-y-4">
        <div>
          <label class="mb-1.5 block text-xs text-[var(--muted)]">任务名称</label>
          <Input v-model="form.name" placeholder="例如：每日配置备份" />
        </div>

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div>
            <label class="mb-1.5 block text-xs text-[var(--muted)]">执行周期</label>
            <Select :model-value="form.period" :options="PERIOD_OPTS" @update:model-value="onPeriodChange" />
          </div>
          <div v-if="form.period === 'weekly'">
            <label class="mb-1.5 block text-xs text-[var(--muted)]">星期几</label>
            <Select v-model="weekDay" :options="WEEK_OPTS" />
          </div>
          <div v-else-if="form.period === 'monthly'">
            <label class="mb-1.5 block text-xs text-[var(--muted)]">每月几号</label>
            <Select v-model="monthDay" :options="MONTH_DAY_OPTS" />
          </div>
        </div>

        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div>
            <label class="mb-1.5 block text-xs text-[var(--muted)]">时</label>
            <Select v-model="hour" :options="HOUR_OPTS" />
          </div>
          <div>
            <label class="mb-1.5 block text-xs text-[var(--muted)]">分</label>
            <Select v-model="minute" :options="minuteOpts" />
          </div>
          <div>
            <label class="mb-1.5 block text-xs text-[var(--muted)]">保留份数</label>
            <Select v-model="keepValue" :options="KEEP_OPTS" />
          </div>
        </div>

        <p class="-mt-1 text-[11px] leading-relaxed text-[var(--muted)]">
          每次执行成功后自动删除更早的备份文件。
        </p>

        <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-[var(--card-soft)] px-3 py-2.5">
          <div class="min-w-0">
            <p class="text-sm">启用此任务</p>
            <p class="mt-0.5 text-[11px] text-[var(--muted)]">关闭后不会自动执行，仍可手动点「立即执行」。</p>
          </div>
          <Switch :checked="form.enabled" @update:checked="(v) => (form.enabled = !!v)" />
        </div>

        <p class="text-[11px] leading-relaxed text-[var(--muted)]">
          备份内容为本机配置目录，生成 .jwbak 压缩包；可在「软件备份」页随时还原。
        </p>
      </div>

      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="editorOpen = false">取消</Button>
          <Button variant="primary" :disabled="saving" @click="save">
            {{ saving ? "保存中…" : "保存" }}
          </Button>
        </div>
      </template>
    </Dialog>

    <!-- 删除确认 -->
    <ConfirmDialog
      :open="!!pendingDelete"
      title="删除定时任务？"
      :message="`将删除「${pendingDelete?.name ?? ''}」，已生成的备份文件不受影响。`"
      confirm-text="删除"
      danger
      @update:open="(v) => !v && (pendingDelete = null)"
      @confirm="doDelete"
    />

    <!-- 清理旧备份：先选保留份数，再确认 -->
    <Dialog :open="cleanupOpen" title="清理旧备份" @update:open="cleanupOpen = $event">
      <div class="space-y-3">
        <p class="text-sm leading-relaxed text-[var(--text)]">
          只会清理「备份导出」目录下的 .jwbak 备份文件，按修改时间保留最新若干份，不会删除子目录。
        </p>
        <div>
          <label class="mb-1.5 block text-xs text-[var(--muted)]">保留最新</label>
          <Select v-model="cleanupKeep" :options="KEEP_OPTS" />
        </div>
      </div>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="cleanupOpen = false">取消</Button>
          <Button
            variant="primary"
            :disabled="cleaning"
            @click="cleanupOpen = false; doCleanup()"
          >
            {{ cleaning ? "清理中…" : "开始清理" }}
          </Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>
