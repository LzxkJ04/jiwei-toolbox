<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { FolderOpen, RefreshCw } from "lucide-vue-next";
import Dialog from "./ui/Dialog.vue";
import Button from "./ui/Button.vue";
import Badge from "./ui/Badge.vue";
import Checkbox from "./ui/Checkbox.vue";
import Select from "./ui/Select.vue";
import Spinner from "./ui/Spinner.vue";
import ConfirmDialog from "./ui/ConfirmDialog.vue";
import {
  cacheDelete, cacheFiles, cacheStats, cleanCache, getDirs,
  type CacheFile, type CacheStats,
} from "../lib/tauri";
import { formatBytes, formatDate } from "../lib/utils";
import { toast } from "../lib/toast";

const props = defineProps<{ open: boolean }>();

const emit = defineEmits<{ (e: "update:open", v: boolean): void }>();

const loading = ref(false);
const busy = ref(false);
const files = ref<CacheFile[]>([]);
const stats = ref<CacheStats | null>(null);
const cacheDir = ref("");
const filter = ref("all");
const selected = ref<string[]>([]);
const confirmDelete = ref(false);
const confirmClean = ref(false);

const filterOpts = [
  { value: "all", label: "全部" },
  { value: "download", label: "下载" },
  { value: "temp", label: "临时" },
  { value: "log", label: "日志" },
];

const kindLabel: Record<string, string> = { download: "下载", temp: "临时", log: "日志" };
const kindTone: Record<string, "primary" | "warning" | "gray"> = {
  download: "primary", temp: "warning", log: "gray",
};

const filtered = computed(() =>
  filter.value === "all"
    ? files.value
    : files.value.filter((f) => f.kind === filter.value)
);

const totalSize = computed(() => stats.value?.total ?? 0);

const allChecked = computed(
  () =>
    filtered.value.length > 0 &&
    filtered.value.every((f) => selected.value.includes(f.name))
);

const selectedSize = computed(() =>
  files.value
    .filter((f) => selected.value.includes(f.name))
    .reduce((s, f) => s + f.size, 0)
);

async function load() {
  loading.value = true;
  selected.value = [];
  const [fs, st, dirs] = await Promise.all([cacheFiles(), cacheStats(), getDirs()]);
  files.value = fs ?? [];
  stats.value = st;
  cacheDir.value = dirs?.cache ?? "";
  loading.value = false;
}

watch(
  () => props.open,
  (v) => {
    if (v) void load();
  }
);

function toggleAll(v: boolean) {
  selected.value = v ? filtered.value.map((f) => f.name) : [];
}

function toggle(name: string, v: boolean) {
  selected.value = v
    ? [...selected.value, name]
    : selected.value.filter((n) => n !== name);
}

async function doDeleteSelected() {
  busy.value = true;
  const ok = await cacheDelete([...selected.value]);
  busy.value = false;
  if (ok !== null) {
    toast(`已删除 ${selected.value.length} 个缓存文件`, "success");
    void load();
  }
}

async function doCleanAll() {
  busy.value = true;
  const ok = await cleanCache();
  busy.value = false;
  if (ok !== null) {
    toast("缓存已全部清空", "success");
    void load();
  }
}
</script>

<template>
  <Dialog
    :open="props.open"
    title="缓存管理"
    class="max-w-2xl"
    @update:open="emit('update:open', $event)"
  >
    <!-- 缓存目录 -->
    <div class="mb-3 flex items-center gap-1.5 text-xs text-[var(--muted)]">
      <FolderOpen class="h-3.5 w-3.5 shrink-0" />
      <span class="break-all">{{ cacheDir || "读取缓存目录…" }}</span>
    </div>

    <!-- 统计行 -->
    <div class="mb-3 flex flex-wrap items-baseline gap-2 rounded-xl bg-[var(--card-soft)] px-4 py-3">
      <span class="text-xl font-bold text-[var(--primary)]">{{ formatBytes(totalSize) }}</span>
      <span class="text-xs text-[var(--muted)]">
        共 {{ files.length }} 个文件 · 已选中 {{ selected.length }} 个（{{ formatBytes(selectedSize) }}）
      </span>
    </div>

    <!-- 筛选 -->
    <div class="mb-3 flex items-center justify-between gap-3">
      <Select v-model="filter" :options="filterOpts" class="w-36" />
    </div>

    <!-- 文件列表 -->
    <div
      class="max-h-80 overflow-y-auto rounded-xl border border-[var(--card-border)]"
    >
      <div
        v-if="loading"
        class="flex items-center justify-center py-10"
      >
        <Spinner label="扫描缓存文件…" />
      </div>
      <table v-else-if="filtered.length" class="w-full text-left text-sm">
        <thead class="sticky top-0 z-10 bg-[var(--card-solid)]/95 text-xs text-[var(--muted)] backdrop-blur">
          <tr>
            <th class="w-9 py-2 pl-3">
              <Checkbox :checked="allChecked" @update:checked="toggleAll" />
            </th>
            <th class="py-2 font-medium">名称</th>
            <th class="py-2 font-medium">大小</th>
            <th class="py-2 font-medium">缓存时间</th>
            <th class="py-2 pr-3 font-medium">类型</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="f in filtered"
            :key="f.name"
            class="border-t border-[var(--card-border)] transition-colors hover:bg-[var(--card-soft)]"
          >
            <td class="py-2 pl-3">
              <Checkbox
                :checked="selected.includes(f.name)"
                @update:checked="(v) => toggle(f.name, v)"
              />
            </td>
            <td class="max-w-56 truncate py-2 pr-2" :title="f.path">{{ f.name }}</td>
            <td class="whitespace-nowrap py-2 pr-2 text-[var(--muted)]">{{ formatBytes(f.size) }}</td>
            <td class="whitespace-nowrap py-2 pr-2 text-xs text-[var(--muted)]">
              {{ f.mtime ? formatDate(f.mtime * 1000) : "-" }}
            </td>
            <td class="py-2 pr-3">
              <Badge :tone="kindTone[f.kind] ?? 'gray'">{{ kindLabel[f.kind] ?? f.kind }}</Badge>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="py-10 text-center text-sm text-[var(--muted)]">
        该分类下暂无缓存文件
      </div>
    </div>

    <template #footer>
      <div class="flex items-center justify-between gap-3">
        <Button variant="ghost" size="sm" :disabled="loading" @click="load">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </Button>
        <div class="flex gap-3">
          <Button
            variant="gray"
            :disabled="busy || loading || selected.length === 0"
            @click="confirmDelete = true"
          >
            删除选中
          </Button>
          <Button
            variant="danger"
            :disabled="busy || loading || files.length === 0"
            @click="confirmClean = true"
          >
            一键清空全部缓存
          </Button>
        </div>
      </div>
    </template>
  </Dialog>

  <ConfirmDialog
    :open="confirmDelete"
    danger
    title="删除选中的缓存文件"
    :message="`将删除选中的 ${selected.length} 个文件（${formatBytes(selectedSize)}），该操作不可恢复。确定继续吗？`"
    confirm-text="删除"
    @confirm="doDeleteSelected"
    @update:open="confirmDelete = $event"
  />

  <ConfirmDialog
    :open="confirmClean"
    danger
    title="清空全部缓存"
    message="将清空全部缓存目录内容（包括下载缓存、临时文件与日志缓存），该操作不可恢复。确定继续吗？"
    confirm-text="全部清空"
    @confirm="doCleanAll"
    @update:open="confirmClean = $event"
  />
</template>
