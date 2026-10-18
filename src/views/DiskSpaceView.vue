<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  ChevronRight, Eraser, FolderOpen, HardDrive, Loader2, Search, Trash2, X,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import {
  cancelScan, deleteSpaceItem, dirUsage, diskOverview, emptyDirContents, emptyRecycleBin,
  revealPath, scanLargeFiles,
  type DiskOverview, type SpaceEntry,
} from "../lib/tauri";
import { formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

type Tab = "overview" | "browser" | "large";
const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "磁盘概览" },
  { id: "browser", label: "目录占用" },
  { id: "large", label: "大文件" },
];
const tab = ref<Tab>("overview");

/* ============ 概览 ============ */

const disks = ref<DiskOverview[]>([]);
const diskLoading = ref(false);

async function loadDisks() {
  diskLoading.value = true;
  disks.value = (await diskOverview()) ?? [];
  diskLoading.value = false;
}

function usedPct(d: DiskOverview) {
  if (!d.total) return 0;
  return Math.min(100, (d.used / d.total) * 100);
}
function barColor(pct: number) {
  if (pct >= 92) return "#e5484d";
  if (pct >= 80) return "#f5a623";
  return "var(--primary)";
}

const driveOpts = computed(() =>
  disks.value
    .map((d) => ({ value: d.mount, label: `${d.mount}  ${formatBytes(d.free)} 可用` }))
    .filter((o) => o.value.length >= 2)
);

function pickDrive(mount: string) {
  void openPath(mount);
}/* ============ 目录浏览 ============ */

const currentPath = ref("");
const entries = ref<SpaceEntry[]>([]);
const entriesLoading = ref(false);
const pathInput = ref("");
const pathTrail = ref<string[]>([]);

const totalShown = computed(() => entries.value.reduce((s, e) => s + e.size, 0));
const maxSize = computed(() => Math.max(1, ...entries.value.map((e) => e.size)));

function splitTrail(p: string) {
  const parts = p.replace(/\//g, "\\").split("\\").filter(Boolean);
  return parts.map((_, i) => parts.slice(0, i + 1).join("\\"));
}

function trailLabel(seg: string) {
  const parts = seg.split("\\").filter(Boolean);
  return parts[parts.length - 1] || seg;
}

async function openPath(p: string) {
  const path = p.trim();
  if (!path) return;
  entriesLoading.value = true;
  entries.value = [];
  try {
    const list = await dirUsage(path, 60);
    currentPath.value = path;
    pathInput.value = path;
    pathTrail.value = splitTrail(path);
    entries.value = list ?? [];
  } catch (err) {
    toast(`读取失败：${String(err)}`, "error");
    entriesLoading.value = false;
    return;
  }
  entriesLoading.value = false;
}

function enter(entry: SpaceEntry) {
  if (entry.kind === "dir") void openPath(entry.path);
}

function goUp() {
  const trail = pathTrail.value;
  if (trail.length <= 1) return;
  void openPath(trail[trail.length - 2]);
}

async function reveal(p: string) {
  try {
    await revealPath(p);
  } catch (err) {
    toast(String(err), "error");
  }
}

/* ============ 清空目录内容 ============ */

const pendingEmpty = ref<SpaceEntry | null>(null);
const emptying = ref(false);

async function confirmEmpty() {
  const item = pendingEmpty.value;
  pendingEmpty.value = null;
  if (!item) return;
  emptying.value = true;
  try {
    const freed = await emptyDirContents(item.path);
    toast(`已清空 ${item.name}，释放 ${formatBytes(freed)}`, "success");
    void openPath(currentPath.value);
  } catch (err) {
    toast(`清空失败：${String(err)}`, "error");
  } finally {
    emptying.value = false;
  }
}

/* ============ 删除确认 ============ */

const pendingDelete = ref<SpaceEntry | null>(null);

async function confirmDelete() {
  const item = pendingDelete.value;
  pendingDelete.value = null;
  if (!item) return;
  try {
    await deleteSpaceItem(item.path);
    toast(`已删除 ${item.name}`, "success");
    entries.value = entries.value.filter((e) => e.path !== item.path);
    largeFiles.value = largeFiles.value.filter((e) => e.path !== item.path);
  } catch (err) {
    toast(`删除失败：${String(err)}`, "error");
  }
}

/* ============ 大文件 ============ */

const largeRoot = ref("");
const largeMinMb = ref("100");
const largeFiles = ref<SpaceEntry[]>([]);
const largeLoading = ref(false);
const largeScanned = ref(false);

const MIN_OPTS = [
  { value: "50", label: "≥ 50 MB" },
  { value: "100", label: "≥ 100 MB" },
  { value: "500", label: "≥ 500 MB" },
  { value: "1024", label: "≥ 1 GB" },
];

async function runLarge() {
  const root = (largeRoot.value || currentPath.value || "").trim();
  if (!root) {
    toast("请先选择或填写要扫描的目录", "warning");
    return;
  }
  largeLoading.value = true;
  largeFiles.value = [];
  largeScanned.value = false;
  try {
    const list = await scanLargeFiles(root, Number(largeMinMb.value), 300);
    largeFiles.value = list ?? [];
    largeScanned.value = true;
  } catch (err) {
    toast(String(err), "error");
  } finally {
    largeLoading.value = false;
  }
}

async function stopScan() {
  await cancelScan();
  toast("已请求停止扫描", "info");
}

const largeTotal = computed(() => largeFiles.value.reduce((s, e) => s + e.size, 0));

/* ============ 回收站 ============ */

const showBin = ref(false);
const binBusy = ref(false);

async function doEmptyBin() {
  showBin.value = false;
  binBusy.value = true;
  try {
    await emptyRecycleBin();
    toast("回收站已清空", "success");
    await loadDisks();
  } catch (err) {
    toast(String(err), "error");
  } finally {
    binBusy.value = false;
  }
}

/* ============ 初始化 ============ */

onMounted(async () => {
  await loadDisks();
  const home = disks.value[0]?.mount || "C:\\";
  largeRoot.value = home;
});

onUnmounted(() => {
  void cancelScan();
});
</script>

<template>
  <div>
    <PageHeader title="磁盘空间" desc="看哪个目录最占地方，揪出大文件，该删的直接删。">
      <template #extra>
        <Button variant="gray" size="sm" :disabled="binBusy" @click="showBin = true">
          <Loader2 v-if="binBusy" class="h-3.5 w-3.5 animate-spin" />
          <Trash2 v-else class="h-3.5 w-3.5" />
          清空回收站
        </Button>
        <Button variant="gray" size="sm" :disabled="diskLoading" @click="loadDisks">
          <Loader2 v-if="diskLoading" class="h-3.5 w-3.5 animate-spin" />
          <HardDrive v-else class="h-3.5 w-3.5" />
          刷新磁盘
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

    <!-- 概览 -->
    <div v-show="tab === 'overview'" class="grid grid-cols-1 gap-4 md:grid-cols-2">
      <Spinner v-if="diskLoading && !disks.length" label="正在读取磁盘…" />
      <Card v-for="d in disks" :key="d.mount" class="p-4">
        <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
          <div class="flex min-w-0 items-center gap-2">
            <HardDrive class="h-4 w-4 shrink-0 text-[var(--primary)]" />
            <p class="truncate text-sm font-semibold">{{ d.mount }}</p>
            <span class="shrink-0 rounded-md bg-[var(--btn-gray)] px-1.5 py-0.5 text-[10px] text-[var(--muted)]">
              {{ d.fs || d.name }}
            </span>
          </div>
          <Button variant="gray" size="sm" @click="openPath(d.mount); tab = 'browser'">分析</Button>
        </div>
        <div class="mb-1 flex items-baseline justify-between text-xs">
          <span class="tabular-nums text-[var(--text)]">
            {{ formatBytes(d.used) }} / {{ formatBytes(d.total) }}
          </span>
          <span class="tabular-nums text-[var(--muted)]">剩 {{ formatBytes(d.free) }}</span>
        </div>
        <div class="h-2.5 overflow-hidden rounded-full bg-[var(--btn-gray)]">
          <div class="h-full rounded-full transition-[width]" :style="{ width: `${usedPct(d)}%`, background: barColor(usedPct(d)) }" />
        </div>
        <p class="mt-1 text-right text-[10px] text-[var(--muted)]">已用 {{ usedPct(d).toFixed(1) }}%</p>
      </Card>
      <EmptyState v-if="!diskLoading && !disks.length" icon="💽" text="没有读到磁盘" sub="点右上角刷新重试" />
    </div>

    <!-- 目录占用 -->
    <div v-show="tab === 'browser'" class="space-y-3">
      <div class="flex flex-wrap items-center gap-2">
        <Button variant="gray" size="sm" :disabled="pathTrail.length <= 1" @click="goUp">上一级</Button>
        <Select
          :model-value="''"
          :options="driveOpts"
          placeholder="选磁盘"
          class="w-40"
          @update:model-value="pickDrive"
        />
        <div class="min-w-[220px] flex-1">
          <Input v-model="pathInput" placeholder="输入完整路径，如 C:\Users\Administrator\AppData" @keyup.enter="openPath(pathInput)" />
        </div>
        <Button variant="primary" size="sm" :disabled="entriesLoading" @click="openPath(pathInput)">
          <Loader2 v-if="entriesLoading" class="h-3.5 w-3.5 animate-spin" />
          <Search v-else class="h-3.5 w-3.5" />
          分析
        </Button>
      </div>

      <div v-if="pathTrail.length" class="flex flex-wrap items-center gap-1 text-xs">
        <template v-for="(seg, i) in pathTrail" :key="seg">
          <button
            type="button"
            class="max-w-[160px] truncate rounded px-1.5 py-0.5 text-[var(--muted)] hover:bg-[var(--btn-gray)] hover:text-[var(--primary)]"
            :title="seg"
            @click="openPath(seg)"
          >
            {{ trailLabel(seg) }}
          </button>
          <ChevronRight v-if="i < pathTrail.length - 1" class="h-3 w-3 shrink-0 text-[var(--muted)]" />
        </template>
      </div>

      <Spinner v-if="entriesLoading" label="正在统计目录体积…" />
      <EmptyState
        v-else-if="!entries.length"
        icon="📁"
        text="还没有分析任何目录"
        sub="在上面输入路径，或从「磁盘概览」点分析。顶层目录会递归统计体积。"
      />
      <Card v-else class="p-4">
        <div class="mb-3 flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--muted)]">
          <span>{{ entries.length }} 项 · 合计 {{ formatBytes(totalShown) }}</span>
          <span>点文件夹进入下一级</span>
        </div>
        <div class="space-y-2">
          <div
            v-for="e in entries"
            :key="e.path"
            class="group rounded-lg px-2 py-1.5 transition-colors hover:bg-[var(--card-soft)]"
          >
            <div class="flex flex-wrap items-center gap-2">
              <button
                type="button"
                class="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                :class="e.kind === 'dir' && 'cursor-pointer'"
                @click="enter(e)"
              >
                <FolderOpen v-if="e.kind === 'dir'" class="h-3.5 w-3.5 shrink-0 text-[var(--primary)]" />
                <HardDrive v-else class="h-3.5 w-3.5 shrink-0 text-[var(--muted)]" />
                <span class="truncate text-sm" :title="e.path">{{ e.name }}</span>
              </button>
              <span class="shrink-0 tabular-nums text-xs text-[var(--muted)]">
                {{ formatBytes(e.size) }}<template v-if="e.kind === 'dir'"> · {{ e.files }} 文件</template>
              </span>
              <div class="flex shrink-0 gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                <Button variant="gray" size="sm" title="在资源管理器中显示" @click="reveal(e.path)">
                  <FolderOpen class="h-3 w-3" />
                </Button>
                <Button
                  v-if="e.kind === 'dir'"
                  variant="gray"
                  size="sm"
                  title="清空该目录内的所有内容（保留目录本身）"
                  @click="pendingEmpty = e"
                >
                  <Eraser class="h-3 w-3" />
                </Button>
                <Button variant="danger" size="sm" title="删除" @click="pendingDelete = e">
                  <Trash2 class="h-3 w-3" />
                </Button>
              </div>
            </div>
            <div class="mt-1 h-1 overflow-hidden rounded-full bg-[var(--btn-gray)]">
              <div
                class="h-full rounded-full"
                :style="{ width: `${(e.size / maxSize) * 100}%`, background: e.kind === 'dir' ? 'var(--primary)' : '#a78bfa' }"
              />
            </div>
          </div>
        </div>
      </Card>
      <p class="text-[10px] text-[var(--muted)]">
        目录体积为递归统计，最多访问 40 万个条目，超大目录可能低估。删除目录要求目录已空，避免误删整个软件。
      </p>
    </div>

    <!-- 大文件 -->
    <div v-show="tab === 'large'" class="space-y-3">
      <div class="flex flex-wrap items-center gap-2">
        <div class="min-w-[220px] flex-1">
          <Input v-model="largeRoot" placeholder="要扫描的目录，如 C:\Users" />
        </div>
        <Select v-model="largeMinMb" :options="MIN_OPTS" class="w-32" />
        <Button variant="primary" size="sm" :disabled="largeLoading" @click="runLarge">
          <Loader2 v-if="largeLoading" class="h-3.5 w-3.5 animate-spin" />
          <Search v-else class="h-3.5 w-3.5" />
          {{ largeLoading ? "扫描中" : "开始扫描" }}
        </Button>
        <Button v-if="largeLoading" variant="danger" size="sm" @click="stopScan">
          <X class="h-3.5 w-3.5" /> 停止
        </Button>
      </div>

      <Spinner v-if="largeLoading" label="正在扫描大文件，可以点停止…" />
      <EmptyState
        v-else-if="!largeScanned"
        icon="🔍"
        text="扫描指定目录下的大文件"
        sub="按体积降序排列，可直接定位或删除。"
      />
      <EmptyState
        v-else-if="!largeFiles.length"
        icon="✅"
        text="没有超过阈值的大文件"
        sub="可以调低体积阈值再扫一次。"
      />
      <Card v-else class="p-0">
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-[var(--card-border)] px-4 py-2 text-xs text-[var(--muted)]">
          <span>{{ largeFiles.length }} 个文件 · 合计 {{ formatBytes(largeTotal) }}</span>
          <span>点击文件名定位到所在目录</span>
        </div>
        <div class="max-h-[60vh] overflow-y-auto">
          <table class="w-full min-w-[640px] text-left text-xs">
            <thead class="sticky top-0 bg-[var(--card-soft)] text-[var(--muted)] backdrop-blur">
              <tr class="border-b border-[var(--card-border)]">
                <th class="px-3 py-2 font-medium">文件</th>
                <th class="px-3 py-2 text-right font-medium">大小</th>
                <th class="px-3 py-2 font-medium">位置</th>
                <th class="px-3 py-2 text-right font-medium">操作</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-[var(--card-border)]">
              <tr v-for="f in largeFiles" :key="f.path" class="hover:bg-[var(--card-soft)]">
                <td class="max-w-[200px] truncate px-3 py-2 font-medium text-[var(--text-strong)]" :title="f.name">{{ f.name }}</td>
                <td class="px-3 py-2 text-right tabular-nums">{{ formatBytes(f.size) }}</td>
                <td class="max-w-[320px] truncate px-3 py-2 text-[var(--muted)]" :title="f.path">{{ f.path }}</td>
                <td class="whitespace-nowrap px-3 py-2 text-right">
                  <Button variant="gray" size="sm" @click="reveal(f.path)"><FolderOpen class="h-3 w-3" /></Button>
                  <Button variant="danger" size="sm" class="ml-1" @click="pendingDelete = f"><Trash2 class="h-3 w-3" /></Button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </Card>
    </div>

    <ConfirmDialog
      :open="!!pendingDelete"
      title="删除这项？"
      :message="pendingDelete ? `将永久删除 ${pendingDelete.name}（${formatBytes(pendingDelete.size)}），不进回收站，无法恢复。` : ''"
      confirm-text="永久删除"
      danger
      @update:open="(v) => !v && (pendingDelete = null)"
      @confirm="confirmDelete"
    />

    <ConfirmDialog
      :open="showBin"
      title="清空回收站？"
      message="回收站里的所有文件都会被永久删除，无法恢复。清空后通常会释放一部分磁盘空间。"
      confirm-text="清空"
      danger
      @update:open="showBin = $event"
      @confirm="doEmptyBin"
    />

    <ConfirmDialog
      :open="!!pendingEmpty"
      title="清空这个目录？"
      :message="pendingEmpty ? `将删除 ${pendingEmpty.name} 内的全部内容（约 ${formatBytes(pendingEmpty.size)}，${pendingEmpty.files} 个文件），目录本身保留。不进回收站，无法恢复。` : ''"
      confirm-text="清空内容"
      danger
      @update:open="(v) => !v && (pendingEmpty = null)"
      @confirm="confirmEmpty"
    />
  </div>
</template>
