<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { Download, FolderOpen, RefreshCw, Trash2 } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Select from "../components/ui/Select.vue";
import Switch from "../components/ui/Switch.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { clearLogs, listLogs, openLogDir, readLog } from "../lib/tauri";
import { toast } from "../lib/toast";

/* ---- 类型筛选：按行内 [module] 前缀 / 关键字匹配（对应 ops-*.log 操作日志） ---- */
type LogType = "all" | "download" | "install" | "uninstall" | "backup" | "hardware";

const TYPE_OPTIONS: { value: LogType; label: string }[] = [
  { value: "all", label: "全部类型" },
  { value: "download", label: "下载" },
  { value: "install", label: "安装" },
  { value: "uninstall", label: "卸载" },
  { value: "backup", label: "备份&配置" },
  { value: "hardware", label: "硬件检测" },
];

const TYPE_MATCHER: Record<Exclude<LogType, "all">, RegExp> = {
  // 行格式：[HH:MM:SS] [module] action : detail
  download: /\[[^\]]*\]\s*download|\[download\]|开始下载/i,
  install: /\[install\]/i,
  uninstall: /\[uninstall\]/i,
  // 备份&配置：备份 / 缓存 / 目录 / 日志 / 系统工具 / UWP 等配置类模块
  backup: /\[(backup|cache|dirs|logs|systools|uwp)\]/i,
  hardware: /\[(hardware|hw|driver|smart)\]|硬件|驱动|smart/i,
};

/** 颜色分级：错误红 > 警告黄 > 成功绿 > 普通灰 */
function lineClass(line: string): string {
  if (/\berror\b|\bfail|错误|失败|异常|panic|✘/i.test(line)) return "text-[#e5484d]";
  if (/\bwarn\b|\bskip|警告|跳过|重试|超时|timeout/i.test(line)) return "text-[#f59e0b]";
  if (/\bsuccess\b|成功|完成|done\b|✔|\bok\b/i.test(line)) return "text-[#30a46c]";
  return "text-[var(--muted)]";
}

/* ---- 数据 ---- */
const files = ref<string[] | null>(null);
const file = ref("");
const raw = ref("");
const fileLoading = ref(false);
const typeFilter = ref<LogType>("all");
const limit = ref(500);
const live = ref(false);
const boxRef = ref<HTMLElement | null>(null);

let liveTimer: number | null = null;

const allLines = computed(() => raw.value.split(/\r?\n/).filter((l) => l.trim() !== ""));
const filteredLines = computed(() => {
  if (typeFilter.value === "all") return allLines.value;
  const re = TYPE_MATCHER[typeFilter.value];
  return allLines.value.filter((l) => re.test(l));
});
const shownLines = computed(() => filteredLines.value.slice(0, limit.value));

async function loadFiles() {
  const list = await listLogs();
  files.value = list ?? [];
  if (list && list.length > 0 && !list.includes(file.value)) {
    file.value = list[0];
    await loadFile();
  } else if (list && list.length === 0) {
    file.value = "";
    raw.value = "";
  }
}

async function loadFile() {
  if (!file.value) return;
  fileLoading.value = true;
  const text = await readLog(file.value);
  raw.value = text ?? "";
  fileLoading.value = false;
  limit.value = 500;
  scrollToBottom();
}

function scrollToBottom() {
  void nextTick(() => {
    boxRef.value?.scrollTo({ top: boxRef.value.scrollHeight });
  });
}

/* ---- 实时滚动：开启时定时刷新当前日志并自动滚到底 ---- */
function setLive(on: boolean) {
  live.value = on;
  if (liveTimer !== null) {
    window.clearInterval(liveTimer);
    liveTimer = null;
  }
  if (on) {
    liveTimer = window.setInterval(async () => {
      if (!file.value) return;
      const text = await readLog(file.value);
      if (text !== null && text !== raw.value) {
        raw.value = text;
        scrollToBottom();
      }
    }, 5000);
    scrollToBottom();
  }
}

/* ---- 导出日志：Blob + a[download]（WebView2 下可用） ---- */
function exportLog() {
  if (shownLines.value.length === 0) {
    toast("当前没有可导出的日志内容", "info");
    return;
  }
  const data = shownLines.value.join("\n");
  const blob = new Blob(["\ufeff" + data], { type: "text/plain;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `${file.value.replace(/\.log$/i, "") || "日志"}-导出.txt`;
  a.click();
  URL.revokeObjectURL(a.href);
  toast(`已导出 ${shownLines.value.length} 行日志`, "success");
}

/* ---- 清空日志（二次确认） ---- */
const showClear = ref(false);
async function confirmClear() {
  const n = await clearLogs();
  if (n !== null) {
    toast(`已清空日志（${n} 个文件）`, "success");
    file.value = "";
    raw.value = "";
    await loadFiles();
  }
}

async function refresh() {
  await loadFiles();
  toast("日志列表已刷新", "info");
}

onMounted(loadFiles);
onBeforeUnmount(() => setLive(false));
</script>

<template>
  <div>
    <PageHeader title="日志中心" desc="查看程序运行与操作日志，支持筛选、导出与清空">
      <template #extra>
        <Button variant="gray" size="sm" @click="refresh">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </Button>
      </template>
    </PageHeader>

    <!-- 顶部筛选栏 -->
    <div class="mb-4 flex flex-wrap items-center gap-4">
      <div class="flex items-center gap-2">
        <span class="text-xs text-[var(--muted)]">日志文件</span>
        <Select
          :model-value="file"
          :options="(files ?? []).map((f) => ({ value: f, label: f }))"
          placeholder="暂无日志文件"
          class="w-48"
          @update:model-value="(v) => { file = v; void loadFile(); }"
        />
      </div>
      <div class="flex items-center gap-2">
        <span class="text-xs text-[var(--muted)]">类型</span>
        <Select
          :model-value="typeFilter"
          :options="TYPE_OPTIONS"
          class="w-32"
          @update:model-value="(v) => { typeFilter = v as LogType; limit = 500; }"
        />
      </div>
      <label class="flex cursor-pointer items-center gap-2 text-xs text-[var(--muted)]">
        <Switch :checked="live" @update:checked="setLive" />
        实时滚动
      </label>
    </div>

    <!-- 日志内容区 -->
    <Card class="p-0">
      <div v-if="files === null" class="p-6">
        <Spinner label="正在读取日志列表…" />
      </div>
      <div v-else-if="!file || allLines.length === 0" class="p-6">
        <EmptyState icon="📄" text="暂无日志内容" sub="程序操作后会生成 ops-*.log 操作日志" />
      </div>
      <template v-else>
        <div
          ref="boxRef"
          class="max-h-[60vh] overflow-y-auto rounded-2xl bg-[var(--card-soft)] p-4 font-mono text-xs leading-relaxed"
        >
          <div v-if="fileLoading" class="py-2 text-center text-[var(--muted)]">加载中…</div>
          <div v-for="(line, i) in shownLines" :key="i" class="whitespace-pre-wrap break-all" :class="lineClass(line)">
            {{ line }}
          </div>
          <div v-if="filteredLines.length > limit" class="py-3 text-center">
            <Button variant="gray" size="sm" @click="limit += 500">
              加载更多（还有 {{ filteredLines.length - limit }} 行）
            </Button>
          </div>
        </div>
      </template>
    </Card>

    <!-- 底部操作栏 -->
    <div class="mt-3 flex flex-wrap items-center gap-3 px-1 text-xs text-[var(--muted)]">
      <span>共 {{ allLines.length }} 行 · 筛选后 {{ filteredLines.length }} 行 · 显示 {{ shownLines.length }} 行</span>
      <div class="ml-auto flex items-center gap-2">
        <Button variant="gray" size="sm" @click="openLogDir">
          <FolderOpen class="h-3.5 w-3.5" /> 打开日志目录
        </Button>
        <Button variant="gray" size="sm" @click="exportLog">
          <Download class="h-3.5 w-3.5" /> 导出日志
        </Button>
        <Button variant="danger" size="sm" :disabled="files !== null && files.length === 0" @click="showClear = true">
          <Trash2 class="h-3.5 w-3.5" /> 清空日志
        </Button>
      </div>
    </div>

    <!-- 清空日志确认 -->
    <ConfirmDialog
      :open="showClear"
      title="清空日志"
      message="确定要删除日志目录下的全部日志文件吗？该操作不可撤销。"
      confirm-text="清空"
      danger
      @update:open="showClear = $event"
      @confirm="confirmClear"
    />
  </div>
</template>
