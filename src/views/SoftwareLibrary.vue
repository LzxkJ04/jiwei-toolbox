<script setup lang="ts">
import { computed, ref } from "vue";
import { ListChecks, ListFilter, Search } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import Dialog from "../components/ui/Dialog.vue";
import SoftwareCard from "../components/SoftwareCard.vue";
import SoftwareDetailDialog from "../components/SoftwareDetailDialog.vue";
import type { SoftwareItem } from "../lib/tauri";
import { useSoftwareStore } from "../stores/software";
import { useFavoritesStore } from "../stores/favorites";
import { useTasksStore } from "../stores/tasks";
import { toast } from "../lib/toast";

const store = useSoftwareStore();
const favorites = useFavoritesStore();
const tasks = useTasksStore();

const search = ref("");
const filter = ref("all");
const category = ref("全部");
/** 排序：默认 / 体积 / 名称 */
const sortBy = ref("default");
const sortOptions = [
  { value: "default", label: "默认顺序" },
  { value: "size", label: "体积小到大" },
  { value: "size-desc", label: "体积大到小" },
  { value: "name", label: "按名称" },
];
/** 勾选状态存于 software store（备份页加载清单可跨页联动） */
const checked = computed(() => store.checked);
const detailItem = ref<SoftwareItem | null>(null);
const detailOpen = ref(false);

const CATEGORIES = [
  "全部", "浏览器", "办公套件", "开发工具", "压缩工具", "多媒体",
  "图片设计", "系统增强", "驱动工具", "实用小工具",
];

const filterOptions = [
  { value: "all", label: "全部软件" },
  { value: "not-installed", label: "仅未安装" },
  { value: "fav", label: "仅收藏" },
  { value: "open-free", label: "开源免费" },
];

const visible = computed(() => {
  let list = store.items ?? [];
  if (category.value !== "全部") list = list.filter((i) => i.category === category.value);
  const q = search.value.trim().toLowerCase();
  if (q) {
    // 多关键词以空格分隔，全部命中才算匹配（如「压缩 免费」）
    const terms = q.split(/\s+/).filter(Boolean);
    list = list.filter((i) => {
      const hay = `${i.name} ${i.notes} ${i.description ?? ""} ${i.category} ${i.license ?? ""} ${i.homepage ?? ""}`.toLowerCase();
      return terms.every((t) => hay.includes(t));
    });
  }
  if (filter.value === "not-installed") list = list.filter((i) => store.statusOf(i) !== "installed");
  else if (filter.value === "fav") list = list.filter((i) => favorites.isFavorite(i.id));
  else if (filter.value === "open-free") list = list.filter((i) => store.licenseOf(i) !== "commercial");

  if (sortBy.value === "size") return list.slice().sort((a, b) => (a.size_mb || 0) - (b.size_mb || 0));
  if (sortBy.value === "size-desc") return list.slice().sort((a, b) => (b.size_mb || 0) - (a.size_mb || 0));
  if (sortBy.value === "name") return list.slice().sort((a, b) => a.name.localeCompare(b.name, "zh-CN"));
  return list;
});

/** 每个分类的数量，显示在标签上 */
const categoryCount = computed(() => {
  const map = new Map<string, number>();
  for (const i of store.items ?? []) {
    map.set(i.category || "其他", (map.get(i.category || "其他") ?? 0) + 1);
  }
  map.set("全部", (store.items ?? []).length);
  return map;
});

/** 当前筛选结果的总体积估算 */
const visibleSize = computed(() => visible.value.reduce((s, i) => s + (i.size_mb || 0), 0));

function setChecked(item: SoftwareItem, v: boolean) {
  store.setChecked(item.id, v);
}

const allChecked = computed(
  () => visible.value.length > 0 && visible.value.every((i) => checked.value.has(i.id))
);

function toggleSelectAll() {
  if (allChecked.value) store.clearChecked();
  else store.addChecked(visible.value.map((i) => i.id));
}

function openDetail(item: SoftwareItem) {
  detailItem.value = item;
  detailOpen.value = true;
}

function addToQueue() {
  if (checked.value.size === 0) return;
  const items = (store.items ?? []).filter((i) => checked.value.has(i.id));
  const n = tasks.addFromSoftware(items);
  if (n === 0) {
    toast("所选软件均已在任务队列中", "info");
    return;
  }
  // 依赖检查：缺依赖单独提示，给一键补齐入口
  const missingDeps = tasks.missingDependencies(items, store.items ?? []);
  if (missingDeps.length > 0) {
    pendingDeps.value = missingDeps;
    toast(`已将 ${n} 个软件加入任务队列，但有 ${missingDeps.length} 个依赖未安装`, "warning");
    return;
  }
  toast(`已将 ${n} 个软件加入任务队列`, "success");
}

/** 缺失依赖提示 */
const pendingDeps = ref<SoftwareItem[]>([]);

function addDepsToQueue() {
  const n = tasks.addFromSoftware(pendingDeps.value);
  pendingDeps.value = [];
  if (n > 0) toast(`已补入 ${n} 个依赖，建议先安装依赖`, "success");
}

/** 导出清单：勾选 id 列表下载为 JSON */
function exportList() {
  if (checked.value.size === 0) {
    toast("请先勾选要导出的软件", "info");
    return;
  }
  const data = JSON.stringify({ type: "jw-software-list", ids: Array.from(checked.value) }, null, 2);
  const blob = new Blob([data], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "软件清单.json";
  a.click();
  URL.revokeObjectURL(a.href);
  toast("清单已导出", "success");
}

/** 导入清单：解析 JSON 并勾选匹配项 */
function importList(e: Event) {
  const file = (e.target as HTMLInputElement).files?.[0];
  (e.target as HTMLInputElement).value = "";
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => {
    try {
      const data = JSON.parse(String(reader.result));
      const ids: string[] = Array.isArray(data) ? data : data?.ids;
      if (!Array.isArray(ids)) throw new Error("格式不符");
      const items = store.items ?? [];
      const matched = new Set(ids.filter((id) => items.some((i) => i.id === id)));
      if (matched.size === 0) {
        toast("清单中没有匹配到本地软件", "error");
        return;
      }
      store.addChecked([...checked.value, ...matched]);
      toast(`已导入并勾选 ${matched.size} 个软件`, "success");
    } catch {
      toast("清单文件解析失败", "error");
    }
  };
  reader.readAsText(file);
}
</script>

<template>
  <div>
    <PageHeader title="软件库" desc="勾选需要的软件，加入任务队列批量静默安装" />

    <!-- 顶部筛选栏 -->
    <div class="mb-4 flex flex-wrap items-center gap-3">
      <div class="relative min-w-[220px] flex-1">
        <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
        <Input v-model="search" placeholder="搜索名称、简介、分类，空格分隔多个关键词" class="!pl-9" />
      </div>
      <div class="flex items-center gap-2">
        <ListFilter class="h-4 w-4 text-[var(--muted)]" />
        <Select v-model="filter" :options="filterOptions" class="w-36" />
      </div>
      <Select v-model="sortBy" :options="sortOptions" class="w-36" />
    </div>

    <!-- 分类标签栏（横向滚动） -->
    <div class="no-scrollbar mb-2 flex gap-2 overflow-x-auto pb-1">
      <button
        v-for="c in CATEGORIES"
        :key="c"
        class="chip"
        :class="category === c && 'chip-active'"
        @click="category = c"
      >
        {{ c }}
        <span v-if="categoryCount.get(c)" class="ml-1 opacity-60">{{ categoryCount.get(c) }}</span>
      </button>
    </div>
    <p class="mb-4 text-xs text-[var(--muted)]">
      {{ visible.length }} 个软件<template v-if="visibleSize > 0"> · 安装包合计约 {{ visibleSize }} MB</template>
    </p>

    <!-- 软件卡片网格 -->
    <div v-if="store.items === null" class="glass-card">
      <Spinner label="正在获取软件列表…" />
    </div>
    <div v-else-if="visible.length === 0" class="glass-card">
      <EmptyState text="没有匹配的软件" sub="试试调整搜索词、筛选条件或分类" />
    </div>
    <div v-else class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
      <SoftwareCard
        v-for="item in visible"
        :key="item.id"
        :item="item"
        :checked="checked.has(item.id)"
        @update:checked="(v) => setChecked(item, v)"
        @detail="openDetail(item)"
      />
    </div>

    <!-- 底部操作栏 -->
    <div
      class="sticky bottom-4 mt-5 flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-[var(--card-border)] bg-[var(--card-solid)]/90 px-5 py-3.5 shadow-glass backdrop-blur-xl"
    >
      <span class="text-sm text-[var(--muted)]">
        已勾选 <b class="text-[var(--primary)]">{{ checked.size }}</b> 个软件
      </span>
      <div class="flex flex-wrap items-center gap-2">
        <Button variant="gray" size="sm" @click="toggleSelectAll">
          {{ allChecked ? "取消全选" : "全选" }}
        </Button>
        <Button variant="gray" size="sm" @click="exportList">导出清单</Button>
        <label class="btn btn-gray !px-3 !py-1.5 text-xs cursor-pointer">
          导入清单
          <input type="file" accept=".json,application/json" class="hidden" @change="importList" />
        </label>
        <Button variant="primary" :disabled="checked.size === 0" @click="addToQueue">
          <ListChecks class="h-4 w-4" />
          加入任务队列
        </Button>
      </div>
    </div>

    <!-- 详情弹窗 -->
    <SoftwareDetailDialog v-model:open="detailOpen" :item="detailItem" />

    <!-- 缺失依赖提示 -->
    <Dialog
      :open="pendingDeps.length > 0"
      title="缺少依赖"
      class="!max-w-md"
      @update:open="(v) => !v && (pendingDeps = [])"
    >
      <p class="text-sm leading-relaxed">
        刚加入的软件依赖以下组件，但本机尚未安装、队列里也没有：
      </p>
      <div class="mt-3 space-y-1 rounded-xl border border-[#f5a62355] bg-[#f5a6231a] p-3">
        <p v-for="d in pendingDeps" :key="d.id" class="text-xs text-[#b57d10]">· {{ d.name }}</p>
      </div>
      <p class="mt-2 text-xs text-[var(--muted)]">
        依赖缺失时主软件可能装上却无法启动。建议一并安装。
      </p>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="pendingDeps = []">忽略</Button>
          <Button variant="primary" @click="addDepsToQueue">一并加入（{{ pendingDeps.length }}）</Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>
