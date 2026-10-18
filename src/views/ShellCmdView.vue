<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  ChevronDown, ChevronRight, Copy, Loader2, Play, Plus, RefreshCw, Save,
  ShieldAlert, Terminal, Trash2, X,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Badge from "../components/ui/Badge.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import {
  listCustomCommands, listPresetCommands, runCustomCommand, runPresetCommand,
  saveCustomCommands,
  type CmdResult, type CustomCommand, type PresetCmd,
} from "../lib/tauri";
import { copyText, formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

const presets = ref<PresetCmd[] | null>(null);
const customs = ref<CustomCommand[]>([]);
const running = ref<string | null>(null);
const result = ref<CmdResult | null>(null);
const resultLabel = ref("");
const showErr = ref(true);

/* 自定义命令编辑 */
const draft = ref<CustomCommand>({ name: "", command: "", note: "" });
const editing = ref(-1);
const pendingDelete = ref(-1);
const pendingRun = ref<CustomCommand | null>(null);

function fmtMs(ms: number) {
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} 秒`;
}

async function load() {
  presets.value = (await listPresetCommands()) ?? [];
  customs.value = (await listCustomCommands()) ?? [];
}

async function runPreset(p: PresetCmd) {
  if (running.value) return;
  running.value = p.id;
  resultLabel.value = p.label;
  result.value = null;
  try {
    result.value = await runPresetCommand(p.id);
    if (result.value.timed_out) toast(`「${p.label}」超时被终止`, "warning");
  } catch (err) {
    toast(`执行失败：${String(err)}`, "error");
  } finally {
    running.value = null;
  }
}

function askRunCustom(c: CustomCommand) {
  pendingRun.value = c;
}

async function doRunCustom() {
  const c = pendingRun.value;
  pendingRun.value = null;
  if (!c || running.value) return;
  running.value = `custom:${c.name}`;
  resultLabel.value = c.name;
  result.value = null;
  try {
    result.value = await runCustomCommand(c.command, 300);
    if (result.value.timed_out) toast("命令超时被终止（上限 5 分钟）", "warning");
  } catch (err) {
    toast(`执行失败：${String(err)}`, "error");
  } finally {
    running.value = null;
  }
}

function editItem(c: CustomCommand, i: number) {
  editing.value = i;
  draft.value = { ...c };
}

function resetDraft() {
  editing.value = -1;
  draft.value = { name: "", command: "", note: "" };
}

async function persist(next: CustomCommand[]) {
  const ok = await saveCustomCommands(next);
  if (ok === null) {
    toast("保存失败", "error");
    return false;
  }
  customs.value = next;
  return true;
}

async function commitDraft() {
  const name = draft.value.name.trim();
  const command = draft.value.command.trim();
  if (!name || !command) {
    toast("名称和命令都不能为空", "warning");
    return;
  }
  const item: CustomCommand = {
    name,
    command,
    note: draft.value.note?.trim() || "",
  };
  const next = [...customs.value];
  if (editing.value >= 0) next[editing.value] = item;
  else next.push(item);
  if (await persist(next)) {
    toast(editing.value >= 0 ? "已更新命令" : "已添加命令", "success");
    resetDraft();
  }
}

async function removeItem() {
  const i = pendingDelete.value;
  pendingDelete.value = -1;
  if (i < 0) return;
  const next = customs.value.filter((_, idx) => idx !== i);
  if (await persist(next)) {
    toast("已删除", "success");
    if (editing.value === i) resetDraft();
  }
}

async function copyOutput() {
  if (!result.value) return;
  const body = [
    `# ${resultLabel.value}`,
    `退出码 ${result.value.code} · 耗时 ${fmtMs(result.value.elapsed_ms)}`,
    "",
    result.value.stdout || "(无输出)",
    result.value.stderr ? `\n--- stderr ---\n${result.value.stderr}` : "",
  ].join("\n");
  const ok = await copyText(body);
  toast(ok ? "输出已复制" : "复制失败", ok ? "success" : "error");
}

const outputSize = computed(() => {
  if (!result.value) return 0;
  return new Blob([result.value.stdout, result.value.stderr]).size;
});

onMounted(load);
</script>

<template>
  <div>
    <PageHeader title="快捷命令" desc="不用开终端，直接跑常用维护命令；也可以保存自己的命令。">
      <template #extra>
        <Button variant="gray" size="sm" :disabled="running !== null" @click="load">
          <RefreshCw class="h-3.5 w-3.5" :class="running && 'animate-spin'" /> 刷新
        </Button>
      </template>
    </PageHeader>

    <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <!-- 左：预设命令 -->
      <Card class="p-5">
        <div class="mb-3 flex items-center justify-between">
          <h4 class="flex items-center gap-2 font-semibold">
            <Terminal class="h-4 w-4 text-[var(--primary)]" /> 预设命令
          </h4>
          <span class="text-xs text-[var(--muted)]">{{ presets?.length ?? 0 }} 项</span>
        </div>

        <Spinner v-if="presets === null" label="加载预设…" />
        <div v-else class="space-y-2">
          <div
            v-for="p in presets"
            :key="p.id"
            class="flex items-start gap-3 rounded-xl border border-[var(--card-border)] p-3 transition-colors hover:border-[var(--primary)]"
          >
            <div class="min-w-0 flex-1">
              <p class="flex flex-wrap items-center gap-1.5 text-sm font-medium">
                {{ p.label }}
                <Badge v-if="p.admin" tone="warning" class="!px-1.5 !py-0 !text-[10px]">
                  <ShieldAlert class="mr-0.5 inline h-3 w-3" />管理员
                </Badge>
              </p>
              <p class="mt-0.5 text-xs leading-relaxed text-[var(--muted)]">{{ p.desc }}</p>
              <p class="mt-1 text-[10px] text-[var(--muted)]">{{ p.hint }}</p>
            </div>
            <Button
              variant="gray"
              size="sm"
              class="shrink-0"
              :disabled="running !== null"
              @click="runPreset(p)"
            >
              <Loader2 v-if="running === p.id" class="h-3.5 w-3.5 animate-spin" />
              <Play v-else class="h-3.5 w-3.5" />
              运行
            </Button>
          </div>
        </div>

        <p class="mt-3 rounded-lg bg-[#f5a6231a] p-2.5 text-[11px] leading-relaxed text-[#b57d10]">
          带「管理员」标记的命令在普通权限下可能提示「拒绝访问」，属正常现象。
          预设命令的参数由程序内置，不接受外部传入。
        </p>
      </Card>

      <!-- 右：自定义命令 -->
      <Card class="p-5">
        <div class="mb-3 flex items-center justify-between">
          <h4 class="font-semibold">我的命令</h4>
          <span class="text-xs text-[var(--muted)]">{{ customs.length }}/100</span>
        </div>

        <!-- 编辑区 -->
        <div class="rounded-xl border border-[var(--card-border)] p-3">
          <div class="flex flex-col gap-2 sm:flex-row">
            <Input v-model="draft.name" placeholder="名称，如「查看路由表」" class="sm:max-w-[160px]" />
            <Input v-model="draft.command" placeholder="命令，如 route print" class="font-mono" />
          </div>
          <Input v-model="draft.note" placeholder="备注（可选）" class="mt-2" />
          <div class="mt-2 flex justify-end gap-2">
            <Button v-if="editing >= 0" variant="gray" size="sm" @click="resetDraft">
              <X class="h-3.5 w-3.5" /> 取消
            </Button>
            <Button variant="primary" size="sm" @click="commitDraft">
              <Save v-if="editing >= 0" class="h-3.5 w-3.5" />
              <Plus v-else class="h-3.5 w-3.5" />
              {{ editing >= 0 ? "保存修改" : "添加" }}
            </Button>
          </div>
        </div>

        <div v-if="customs.length === 0" class="mt-3">
          <EmptyState icon="⌨" text="还没有自定义命令" sub="上面填名称和命令即可保存，命令通过 cmd /c 执行" />
        </div>
        <div v-else class="mt-3 space-y-2">
          <div
            v-for="(c, i) in customs"
            :key="`${c.name}-${i}`"
            class="group flex items-start gap-2 rounded-xl border border-[var(--card-border)] p-3"
          >
            <div class="min-w-0 flex-1">
              <p class="truncate text-sm font-medium">{{ c.name }}</p>
              <p class="mt-0.5 truncate font-mono text-[11px] text-[var(--muted)]" :title="c.command">
                {{ c.command }}
              </p>
              <p v-if="c.note" class="mt-0.5 truncate text-[10px] text-[var(--muted)]">{{ c.note }}</p>
            </div>
            <div class="flex shrink-0 gap-1">
              <Button
                variant="gray"
                size="sm"
                :disabled="running !== null"
                title="运行（会二次确认）"
                @click="askRunCustom(c)"
              >
                <Loader2 v-if="running === `custom:${c.name}`" class="h-3.5 w-3.5 animate-spin" />
                <Play v-else class="h-3.5 w-3.5" />
              </Button>
              <Button variant="gray" size="sm" title="编辑" @click="editItem(c, i)">
                <ChevronRight class="h-3.5 w-3.5" />
              </Button>
              <Button variant="danger" size="sm" title="删除" @click="pendingDelete = i">
                <Trash2 class="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
        </div>
      </Card>
    </div>

    <!-- 输出 -->
    <Card class="mt-4 p-5">
      <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h4 class="flex items-center gap-2 font-semibold">
          <ChevronDown class="h-4 w-4 text-[var(--muted)]" />
          执行输出
          <span v-if="resultLabel" class="text-xs font-normal text-[var(--muted)]">· {{ resultLabel }}</span>
        </h4>
        <div class="flex flex-wrap items-center gap-2">
          <Badge
            v-if="result"
            :tone="result.timed_out ? 'warning' : result.code === 0 ? 'success' : 'danger'"
          >
            {{ result.timed_out ? "超时终止" : result.code === 0 ? "成功" : `退出码 ${result.code}` }}
          </Badge>
          <span v-if="result" class="text-[11px] text-[var(--muted)]">
            耗时 {{ fmtMs(result.elapsed_ms) }} · {{ formatBytes(outputSize) }}
          </span>
          <Button v-if="result" variant="gray" size="sm" @click="copyOutput">
            <Copy class="h-3.5 w-3.5" /> 复制
          </Button>
        </div>
      </div>

      <EmptyState
        v-if="!result"
        icon="▶"
        text="还没有执行任何命令"
        sub="左侧点「运行」，或添加自己的命令后运行"
      />
      <template v-else>
        <div class="mb-2 flex flex-wrap gap-2 text-xs">
          <button
            class="chip"
            :class="showErr && 'chip-active'"
            @click="showErr = true"
          >
            标准输出
          </button>
          <button class="chip" :class="!showErr && 'chip-active'" @click="showErr = false">
            stderr{{ result.stderr ? ` (${result.stderr.split('\n').length})` : "" }}
          </button>
        </div>
        <pre
          class="max-h-[420px] overflow-auto whitespace-pre-wrap break-all rounded-xl bg-[var(--btn-gray)] p-3 font-mono text-xs leading-relaxed"
        >{{ showErr ? (result.stdout || "(无输出)") : (result.stderr || "(无错误输出)") }}</pre>
        <p v-if="result.truncated" class="mt-2 text-[11px] text-[#b57d10]">
          输出过长已截断，仅保留前 200 KB。
        </p>
      </template>
    </Card>

    <!-- 自定义命令执行确认 -->
    <ConfirmDialog
      :open="!!pendingRun"
      title="执行自定义命令"
      :message="pendingRun ? `即将执行：\n${pendingRun.command}\n\n命令以当前用户权限运行，请在确认内容来源可靠后再继续。` : ''"
      confirm-text="执行"
      @update:open="(v) => !v && (pendingRun = null)"
      @confirm="doRunCustom"
    />

    <ConfirmDialog
      :open="pendingDelete >= 0"
      title="删除这条命令？"
      :message="pendingDelete >= 0 ? `将移除「${customs[pendingDelete]?.name ?? ''}」，只删除保存记录，不影响系统。` : ''"
      confirm-text="删除"
      danger
      @update:open="(v) => !v && (pendingDelete = -1)"
      @confirm="removeItem"
    />
  </div>
</template>
