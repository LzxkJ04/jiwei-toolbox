<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { ChevronDown, ChevronRight, Copy, RefreshCw, Trash2 } from "lucide-vue-next";
import { invoke } from "@tauri-apps/api/core";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Badge from "../components/ui/Badge.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { copyText, formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

/* ------------------------------------------------------------------ 类型 */

type Level = "INFO" | "WARN" | "ERROR" | "FATAL";

interface LogEntry {
  timestamp: string;
  level: Level;
  module: string;
  error_code: string;
  message: string;
  detail: string;
  task_id: string;
  status: string;
  is_admin: boolean;
  os_info: string;
}

interface LogStats {
  info: number;
  warn: number;
  error: number;
  fatal: number;
  total: number;
}

/* ------------------------------------------------------------------ 日期 */

const pad = (n: number) => String(n).padStart(2, "0");

function fmtDate(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const today = fmtDate(new Date());
const date = ref(today);

/** 最近 7 天快捷按钮（今天在最前） */
const recentDays = computed(() => {
  const base = new Date();
  const out: { value: string; label: string }[] = [];
  for (let i = 0; i < 7; i += 1) {
    const d = new Date(base.getFullYear(), base.getMonth(), base.getDate() - i);
    out.push({
      value: fmtDate(d),
      label: i === 0 ? "今天" : i === 1 ? "昨天" : `${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    });
  }
  return out;
});

/* ------------------------------------------------------------------ 数据 */

const entries = ref<LogEntry[]>([]);
const stats = ref<LogStats>({ info: 0, warn: 0, error: 0, fatal: 0, total: 0 });
const loading = ref(false);
const limit = ref(500);

const level = ref<"ALL" | Level>("ALL");
/** Radix Select 不允许空字符串作为选项值，用哨兵值表示「全部模块」 */
const ALL_MODULES = "__all__";
const moduleFilter = ref(ALL_MODULES);
const keyword = ref("");
const codeQuery = ref("");
const expandedKey = ref<string | null>(null);
const showClear = ref(false);

const emptyStats = (): LogStats => ({ info: 0, warn: 0, error: 0, fatal: 0, total: 0 });

async function load() {
  loading.value = true;
  try {
    const [list, s] = await Promise.all([
      invoke<LogEntry[]>("list_structured_logs", { date: date.value, limit: limit.value }),
      invoke<LogStats>("structured_log_stats", { date: date.value }),
    ]);
    entries.value = Array.isArray(list) ? list : [];
    stats.value = s ? { ...emptyStats(), ...s } : emptyStats();
  } catch (e) {
    entries.value = [];
    stats.value = emptyStats();
    toast(`读取日志失败：${String(e)}`, "error");
  } finally {
    loading.value = false;
  }
}

async function refresh() {
  expandedKey.value = null;
  await load();
  toast(`已刷新 ${date.value} 的日志`, "info");
}

function loadMore() {
  limit.value *= 2;
  void load();
}

/* ------------------------------------------------------------------ 筛选 */

/** 模块下拉：从已加载日志里去重 */
const moduleOptions = computed(() => {
  const set = new Set<string>();
  for (const e of entries.value) {
    const m = (e.module ?? "").trim();
    if (m) set.add(m);
  }
  return [
    { value: ALL_MODULES, label: "全部模块" },
    ...[...set].sort((a, b) => a.localeCompare(b, "zh-CN")).map((m) => ({ value: m, label: m })),
  ];
});

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase();
  const code = codeQuery.value.trim().toLowerCase();
  // 后端按时间正序返回（老→新），界面倒序展示，最近发生的排在最前
  return entries.value
    .filter((e) => {
      if (level.value !== "ALL" && e.level !== level.value) return false;
      if (moduleFilter.value !== ALL_MODULES && e.module !== moduleFilter.value) return false;
      if (code && !(e.error_code ?? "").toLowerCase().includes(code)) return false;
      if (kw) {
        const hay = `${e.message ?? ""}\n${e.detail ?? ""}\n${e.error_code ?? ""}\n${e.module ?? ""}`;
        if (!hay.toLowerCase().includes(kw)) return false;
      }
      return true;
    })
    .reverse();
});

/** 有筛选条件时，空列表提示要区分「这天没日志」和「筛没了」 */
const hasFilter = computed(
  () =>
    level.value !== "ALL" ||
    moduleFilter.value !== ALL_MODULES ||
    keyword.value.trim() !== "" ||
    codeQuery.value.trim() !== ""
);

const levelChips = computed(() => [
  {
    value: "ALL" as const,
    label: "全部",
    count: stats.value.total,
    active: "bg-[var(--primary)] text-white",
    idle: "bg-[var(--btn-gray)] text-[var(--muted)] hover:text-[var(--text)]",
  },
  {
    value: "INFO" as const,
    label: "INFO",
    count: stats.value.info,
    active: "bg-[#3b82f6] text-white",
    idle: "bg-[#3b82f6]/12 text-[#3b82f6] hover:bg-[#3b82f6]/20",
  },
  {
    value: "WARN" as const,
    label: "WARN",
    count: stats.value.warn,
    active: "bg-[#f59e0b] text-white",
    idle: "bg-[#f59e0b]/12 text-[#f59e0b] hover:bg-[#f59e0b]/20",
  },
  {
    value: "ERROR" as const,
    label: "ERROR",
    count: stats.value.error,
    active: "bg-[#e5484d] text-white",
    idle: "bg-[#e5484d]/12 text-[#e5484d] hover:bg-[#e5484d]/20",
  },
  {
    value: "FATAL" as const,
    label: "FATAL",
    count: stats.value.fatal,
    active: "bg-[#b91c1c] text-white",
    idle: "bg-[#b91c1c]/12 text-[#b91c1c] hover:bg-[#b91c1c]/20",
  },
]);

/** 统计条四张卡 */
const statCards = computed(() => {
  const total = stats.value.total || 0;
  const pct = (n: number) => (total > 0 ? Math.round((n / total) * 100) : 0);
  return [
    { key: "info", label: "信息", count: stats.value.info, pct: pct(stats.value.info), text: "text-[#3b82f6]", bar: "bg-[#3b82f6]" },
    { key: "warn", label: "警告", count: stats.value.warn, pct: pct(stats.value.warn), text: "text-[#f59e0b]", bar: "bg-[#f59e0b]" },
    { key: "error", label: "错误", count: stats.value.error, pct: pct(stats.value.error), text: "text-[#e5484d]", bar: "bg-[#e5484d]" },
    { key: "fatal", label: "严重", count: stats.value.fatal, pct: pct(stats.value.fatal), text: "text-[#b91c1c]", bar: "bg-[#b91c1c]" },
  ];
});

/* ------------------------------------------------------------------ 展示 */

function timeOf(ts: string): string {
  if (!ts) return "--:--:--";
  const i = ts.indexOf(" ");
  return i >= 0 ? ts.slice(i + 1) : ts;
}

function levelTone(lv: Level): "primary" | "warning" | "danger" {
  if (lv === "INFO") return "primary";
  if (lv === "WARN") return "warning";
  return "danger";
}

/** FATAL 用深红强调：徽章加粗 + 深红底 */
function levelBadgeClass(lv: Level): string {
  if (lv === "FATAL") return "!bg-[#b91c1c]/14 !text-[#b91c1c] font-bold";
  return "";
}

function rowAccent(lv: Level): string {
  switch (lv) {
    case "INFO":
      return "border-l-2 border-l-[#3b82f6]";
    case "WARN":
      return "border-l-2 border-l-[#f59e0b]";
    case "ERROR":
      return "border-l-2 border-l-[#e5484d]";
    default:
      return "border-l-2 border-l-[#b91c1c] bg-[#b91c1c0d]";
  }
}

/** 展开状态的 key：内容 + 序号，避免同一毫秒内的相同记录互相顶掉 */
const rowKey = (e: LogEntry, i: number): string =>
  `${e.timestamp}|${e.module}|${e.error_code}|${e.message}|${i}`;

function toggle(e: LogEntry, i: number) {
  const key = rowKey(e, i);
  expandedKey.value = expandedKey.value === key ? null : key;
}

/** 关键词高亮切片：不用 v-html，避免 XSS */
function splitHighlight(text: string): { text: string; hit: boolean }[] {
  const src = text ?? "";
  const kw = keyword.value.trim();
  if (!kw) return src ? [{ text: src, hit: false }] : [];
  const lower = src.toLowerCase();
  const k = kw.toLowerCase();
  const out: { text: string; hit: boolean }[] = [];
  let from = 0;
  while (from <= src.length) {
    const at = lower.indexOf(k, from);
    if (at < 0) {
      if (from < src.length) out.push({ text: src.slice(from), hit: false });
      break;
    }
    if (at > from) out.push({ text: src.slice(from, at), hit: false });
    out.push({ text: src.slice(at, at + k.length), hit: true });
    from = at + k.length;
  }
  return out;
}

const entryText = (e: LogEntry): string => {
  const head = `[${e.timestamp}] [${e.level}] [${e.module}]${e.error_code ? ` [${e.error_code}]` : ""} ${e.message}`;
  const lines = [head];
  if (e.status) lines.push(`  状态: ${e.status}`);
  if (e.task_id) lines.push(`  任务: ${e.task_id}`);
  if (e.detail) lines.push(`  详情: ${e.detail.replace(/\r?\n/g, "\n        ")}`);
  if (e.os_info) lines.push(`  系统: ${e.os_info}${e.is_admin ? "（管理员）" : ""}`);
  return lines.join("\n");
};

async function copyOne(e: LogEntry) {
  const ok = await copyText(entryText(e));
  toast(ok ? "这条日志已复制" : "复制失败，请手动选择文本", ok ? "success" : "error");
}

async function copyCode(code: string) {
  if (!code) return;
  const ok = await copyText(code);
  toast(ok ? `已复制 ${code}` : "复制失败", ok ? "success" : "error");
}

async function exportShown() {
  if (filtered.value.length === 0) {
    toast("当前筛选结果为空，没有可导出的日志", "info");
    return;
  }
  const text = filtered.value.map(entryText).join("\n\n");
  const ok = await copyText(text);
  toast(
    ok ? `已复制 ${filtered.value.length} 条日志到剪贴板` : "复制失败，请减少条数后重试",
    ok ? "success" : "error"
  );
}

/* ------------------------------------------------------------------ 清空 */

async function confirmClear() {
  try {
    const bytes = await invoke<number>("clear_structured_logs", { date: date.value });
    if (!bytes) {
      toast("这一天本来就没有日志文件", "info");
    } else {
      toast(`已删除 ${date.value} 的日志（${formatBytes(bytes)}）`, "success");
    }
    expandedKey.value = null;
    await load();
  } catch (e) {
    toast(`清空失败：${String(e)}`, "error");
  }
}

/* ------------------------------------------------------------------ 生命周期 */

// 筛选变化后行序改变，展开态要重置，避免展开到别的行
watch([level, moduleFilter, keyword, codeQuery], () => {
  expandedKey.value = null;
});

watch(date, () => {
  limit.value = 500;
  expandedKey.value = null;
  // 换天后模块集合可能完全不同，清掉模块筛选避免筛出空表
  moduleFilter.value = ALL_MODULES;
  // 先清空，避免加载期间把上一天的日志当成这天的显示
  entries.value = [];
  stats.value = emptyStats();
  void load();
});

onMounted(load);
</script>

<template>
  <div>
    <PageHeader title="结构化日志" desc="按等级、模块、错误码检索结构化运行日志，点任意一行展开原始详情">
      <template #extra>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="gray" size="sm" :disabled="loading" @click="refresh">
            <RefreshCw class="h-3.5 w-3.5" /> 刷新
          </Button>
          <Button variant="gray" size="sm" @click="exportShown">
            <Copy class="h-3.5 w-3.5" /> 导出
          </Button>
          <Button variant="danger" size="sm" :disabled="loading" @click="showClear = true">
            <Trash2 class="h-3.5 w-3.5" /> 清空
          </Button>
        </div>
      </template>
    </PageHeader>

    <!-- 日期 + 快捷 7 天 -->
    <Card class="mb-4 p-5">
      <div class="flex flex-wrap items-end gap-3">
        <div class="flex items-center gap-2">
          <span class="shrink-0 text-xs text-[var(--muted)]">日期</span>
          <Input v-model="date" type="date" class="w-[170px] dark:[color-scheme:dark]" />
        </div>
        <div class="flex flex-wrap items-center gap-1.5">
          <button
            v-for="d in recentDays"
            :key="d.value"
            type="button"
            class="rounded-full px-2.5 py-1 text-xs font-medium transition-colors"
            :class="
              date === d.value
                ? 'bg-[var(--primary)] text-white'
                : 'bg-[var(--btn-gray)] text-[var(--muted)] hover:text-[var(--text)]'
            "
            @click="date = d.value"
          >
            {{ d.label }}
          </button>
        </div>
        <span class="ml-auto text-xs text-[var(--muted)]">
          当天共 <span class="font-medium text-[var(--text)]">{{ stats.total }}</span> 条
        </span>
      </div>

      <!-- 等级 chips -->
      <div class="mt-4 flex flex-wrap items-center gap-2">
        <button
          v-for="c in levelChips"
          :key="c.value"
          type="button"
          class="rounded-full px-3 py-1 text-xs font-medium transition-colors"
          :class="level === c.value ? c.active : c.idle"
          @click="level = c.value"
        >
          {{ c.label }}
          <span class="ml-1 tabular-nums opacity-80">{{ c.count }}</span>
        </button>
      </div>

      <!-- 模块 / 关键词 / 错误码 -->
      <div class="mt-3 flex flex-wrap items-center gap-3">
        <div class="flex items-center gap-2">
          <span class="shrink-0 text-xs text-[var(--muted)]">模块</span>
          <Select v-model="moduleFilter" :options="moduleOptions" class="w-[150px]" />
        </div>
        <div class="flex min-w-[200px] flex-1 items-center gap-2">
          <span class="shrink-0 text-xs text-[var(--muted)]">搜索</span>
          <Input v-model="keyword" placeholder="消息或详情关键词" class="flex-1" />
        </div>
        <div class="flex items-center gap-2">
          <span class="shrink-0 text-xs text-[var(--muted)]">错误码</span>
          <Input v-model="codeQuery" placeholder="如 MOD01" class="w-[140px] font-mono" />
        </div>
      </div>
    </Card>

    <!-- 统计条 -->
    <div class="mb-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
      <Card v-for="s in statCards" :key="s.key" class="p-4">
        <div class="flex flex-wrap items-baseline justify-between gap-1">
          <span class="text-xs text-[var(--muted)]">{{ s.label }}</span>
          <span class="text-[10px] text-[var(--muted)]">{{ s.pct }}%</span>
        </div>
        <p class="mt-1 text-xl font-bold tabular-nums" :class="s.text">{{ s.count }}</p>
        <div class="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-[var(--btn-gray)]">
          <div class="h-full rounded-full transition-all" :class="s.bar" :style="{ width: `${s.pct}%` }" />
        </div>
      </Card>
    </div>

    <!-- 日志表格 -->
    <Card class="p-0">
      <Spinner v-if="loading && entries.length === 0" label="正在读取结构化日志…" />
      <EmptyState
        v-else-if="entries.length === 0"
        icon="📭"
        text="这一天没有日志记录"
        sub="换一个日期看看，或等程序执行任务后再回来。"
      />
      <div v-else class="overflow-x-auto">
        <table class="w-full min-w-[880px] text-left text-xs">
          <thead>
            <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
              <th class="w-[46px] px-2 py-2.5" />
              <th class="w-[110px] px-3 py-2.5 font-medium">时间</th>
              <th class="w-[86px] px-3 py-2.5 font-medium">等级</th>
              <th class="w-[110px] px-3 py-2.5 font-medium">模块</th>
              <th class="w-[118px] px-3 py-2.5 font-medium">错误码</th>
              <th class="px-3 py-2.5 font-medium">消息</th>
              <th class="w-[86px] px-3 py-2.5 font-medium">状态</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-[var(--card-border)]">
            <template v-for="(e, i) in filtered" :key="rowKey(e, i)">
              <tr
                class="cursor-pointer align-top transition-colors hover:bg-[var(--card-soft)]"
                :class="rowAccent(e.level)"
                @click="toggle(e, i)"
              >
                <td class="px-2 py-2 text-[var(--muted)]">
                  <component
                    :is="expandedKey === rowKey(e, i) ? ChevronDown : ChevronRight"
                    class="h-3.5 w-3.5"
                  />
                </td>
                <td class="whitespace-nowrap px-3 py-2 font-mono tabular-nums text-[var(--muted)]" :title="e.timestamp">
                  {{ timeOf(e.timestamp) }}
                </td>
                <td class="px-3 py-2">
                  <Badge :tone="levelTone(e.level)" :class="levelBadgeClass(e.level)">
                    {{ e.level }}
                  </Badge>
                </td>
                <td class="max-w-[110px] truncate px-3 py-2 text-[var(--text)]" :title="e.module">
                  {{ e.module || "—" }}
                </td>
                <td class="px-3 py-2">
                  <button
                    v-if="e.error_code"
                    type="button"
                    class="rounded px-1.5 py-0.5 font-mono text-[11px] text-[var(--primary)] transition-colors hover:bg-[var(--primary-soft)]"
                    :title="`复制 ${e.error_code}`"
                    @click.stop="copyCode(e.error_code)"
                  >
                    {{ e.error_code }}
                  </button>
                  <span v-else class="text-[var(--muted)]">—</span>
                </td>
                <td class="break-all px-3 py-2 text-[var(--text)]">
                  <template v-for="(part, pi) in splitHighlight(e.message)" :key="pi">
                    <mark
                      v-if="part.hit"
                      class="rounded bg-[#f59e0b]/25 px-0.5 text-[var(--text-strong)]"
                    >{{ part.text }}</mark>
                    <template v-else>{{ part.text }}</template>
                  </template>
                </td>
                <td class="max-w-[86px] truncate px-3 py-2 text-[var(--muted)]" :title="e.status">
                  {{ e.status || "—" }}
                </td>
              </tr>

              <!-- 内联展开详情 -->
              <tr v-if="expandedKey === rowKey(e, i)" class="bg-[var(--card-soft)]">
                <td class="px-2 py-3 text-[var(--muted)]" />
                <td colspan="6" class="px-3 py-3">
                  <div class="flex flex-wrap items-center gap-2">
                    <Badge tone="gray">{{ e.module || "未知模块" }}</Badge>
                    <Badge v-if="e.error_code" tone="primary" class="font-mono">{{ e.error_code }}</Badge>
                    <Badge :tone="e.is_admin ? 'success' : 'gray'">{{ e.is_admin ? "管理员" : "普通权限" }}</Badge>
                    <span v-if="e.task_id" class="font-mono text-[10px] text-[var(--muted)]">任务 {{ e.task_id }}</span>
                    <Button variant="ghost" size="sm" class="ml-auto text-[var(--muted)]" @click.stop="copyOne(e)">
                      <Copy class="h-3.5 w-3.5" /> 复制这条
                    </Button>
                  </div>

                  <dl class="mt-3 grid gap-x-6 gap-y-2 text-xs sm:grid-cols-2">
                    <div class="flex min-w-0 flex-wrap gap-1.5">
                      <dt class="shrink-0 text-[var(--muted)]">完整时间</dt>
                      <dd class="font-mono text-[var(--text)]">{{ e.timestamp || "—" }}</dd>
                    </div>
                    <div class="flex min-w-0 flex-wrap gap-1.5">
                      <dt class="shrink-0 text-[var(--muted)]">状态</dt>
                      <dd class="text-[var(--text)]">{{ e.status || "—" }}</dd>
                    </div>
                    <div class="flex min-w-0 flex-wrap gap-1.5">
                      <dt class="shrink-0 text-[var(--muted)]">任务 ID</dt>
                      <dd class="break-all font-mono text-[var(--text)]">{{ e.task_id || "—" }}</dd>
                    </div>
                    <div class="flex min-w-0 flex-wrap gap-1.5">
                      <dt class="shrink-0 text-[var(--muted)]">系统信息</dt>
                      <dd class="break-all text-[var(--text)]">{{ e.os_info || "—" }}</dd>
                    </div>
                  </dl>

                  <div class="mt-3">
                    <p class="mb-1 text-[10px] font-medium text-[var(--muted)]">原始错误</p>
                    <pre
                      v-if="e.detail"
                      class="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] p-2.5 font-mono text-[11px] leading-relaxed text-[var(--text)]"
                    ><template v-for="(part, pi) in splitHighlight(e.detail)" :key="pi"><mark
                      v-if="part.hit"
                      class="rounded bg-[#f59e0b]/25 text-[var(--text-strong)]"
                    >{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></pre>
                    <p v-else class="rounded-lg border border-[var(--card-border)] bg-[var(--card-soft)] p-2.5 text-[11px] text-[var(--muted)]">
                      这条日志没有附带原始错误详情。
                    </p>
                  </div>
                </td>
              </tr>
            </template>

            <tr v-if="filtered.length === 0">
              <td colspan="7" class="px-3 py-8 text-center text-xs text-[var(--muted)]">
                当前筛选条件下没有日志，试试清空关键词或切回「全部」等级。
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </Card>

    <!-- 分页 / 加载更多 -->
    <div class="mt-3 flex flex-wrap items-center gap-3 px-1 text-xs text-[var(--muted)]">
      <span>
        已加载 {{ entries.length }} 条 · 筛选后 {{ filtered.length }} 条
        <template v-if="stats.total > entries.length"> · 当天共 {{ stats.total }} 条</template>
      </span>
      <div class="ml-auto flex flex-wrap items-center gap-2">
        <Button
          v-if="stats.total > entries.length"
          variant="gray"
          size="sm"
          :disabled="loading"
          @click="loadMore"
        >
          {{ loading ? "加载中…" : `加载更多（上限 ${limit * 2} 条）` }}
        </Button>
        <span v-else-if="entries.length > 0">已到当天最早的记录</span>
        <span v-if="hasFilter" class="text-[10px]">筛选在前端完成，不会重复请求后端</span>
      </div>
    </div>

    <ConfirmDialog
      :open="showClear"
      title="清空这一天的日志？"
      :message="`将删除 ${date} 的结构化日志文件，删除后无法恢复。建议先导出需要保留的记录。`"
      confirm-text="清空"
      danger
      @update:open="showClear = $event"
      @confirm="confirmClear"
    />
  </div>
</template>
