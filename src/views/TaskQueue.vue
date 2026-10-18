<script setup lang="ts">
import { computed, ref } from "vue";
import {
  Play, Pause, Square, Trash2, FileText, X, GripVertical, RotateCcw, Timer, Sparkles,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Progress from "../components/ui/Progress.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import Select from "../components/ui/Select.vue";
import Dialog from "../components/ui/Dialog.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { useRouteStore } from "../stores/route";
import {
  useTasksStore, FINISH_ACTIONS,
  type TaskStatus, type TaskKind, type TaskFilter,
} from "../stores/tasks";
import { toast } from "../lib/toast";

const tasks = useTasksStore();
const route = useRouteStore();

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
        <div class="flex items-center gap-2">
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

    <!-- 关机 / 休眠 警告提示 -->
    <div
      v-if="tasks.finishAction === 'shutdown' || tasks.finishAction === 'sleep'"
      class="mb-3 rounded-lg border border-[#f5a623]/40 bg-[#f5a623]/10 px-3 py-2 text-xs text-[#b7791f]"
    >
      ⚠ 已选择任务完成后自动{{ tasks.finishAction === "shutdown" ? "关机" : "休眠" }}：任务完成 60 秒后执行，可 shutdown /a 取消
    </div>

    <!-- 全局控制面板 -->
    <Card v-if="tasks.tasks.length > 0" class="mb-4 p-5">
      <div class="mb-2 flex items-center justify-between text-sm">
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
      <div v-if="tasks.totalElapsed > 0" class="mt-2 flex items-center gap-1 text-xs text-[var(--muted)]">
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
    <div v-if="tasks.tasks.length > 0" class="mb-3 flex items-center gap-2">
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
        class="flex items-center gap-3 p-4 transition-opacity"
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
        <div class="flex shrink-0 items-center gap-1">
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
      <div class="ml-auto flex items-center gap-1">
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

    <!-- 终止全部：二选一 -->
    <Dialog :open="showTerminate" title="终止全部任务" @update:open="showTerminate = $event">
      <p class="text-sm leading-relaxed text-[var(--text)]">
        将终止所有等待中 / 执行中 / 已暂停的任务。已下载的安装包缓存如何处理？
      </p>
      <template #footer>
        <div class="flex justify-end gap-3">
          <Button variant="gray" @click="showTerminate = false">取消</Button>
          <Button variant="gray" @click="confirmTerminate(false)">终止并保留缓存</Button>
          <Button variant="danger" @click="confirmTerminate(true)">终止并删除全部缓存</Button>
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
