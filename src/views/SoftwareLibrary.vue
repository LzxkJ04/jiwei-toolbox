<script setup lang="ts">
import { computed, ref } from "vue";
import { ListChecks, ListFilter, Search } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
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
const checked = ref(new Set<string>());
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
  if (q) list = list.filter((i) => `${i.name} ${i.notes}`.toLowerCase().includes(q));
  if (filter.value === "not-installed") list = list.filter((i) => store.statusOf(i) !== "installed");
  else if (filter.value === "fav") list = list.filter((i) => favorites.isFavorite(i.id));
  else if (filter.value === "open-free") list = list.filter((i) => store.licenseOf(i) !== "commercial");
  return list;
});

function setChecked(item: SoftwareItem, v: boolean) {
  const next = new Set(checked.value);
  if (v) next.add(item.id);
  else next.delete(item.id);
  checked.value = next;
}

const allChecked = computed(
  () => visible.value.length > 0 && visible.value.every((i) => checked.value.has(i.id))
);

function toggleSelectAll() {
  if (allChecked.value) checked.value = new Set();
  else checked.value = new Set(visible.value.map((i) => i.id));
}

function openDetail(item: SoftwareItem) {
  detailItem.value = item;
  detailOpen.value = true;
}

function addToQueue() {
  if (checked.value.size === 0) return;
  const items = (store.items ?? []).filter((i) => checked.value.has(i.id));
  const n = tasks.addFromSoftware(items);
  toast(
    n > 0 ? `已将 ${n} 个软件加入任务队列` : "所选软件均已在任务队列中",
    n > 0 ? "success" : "info"
  );
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
      checked.value = new Set([...checked.value, ...matched]);
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
        <Input v-model="search" placeholder="搜索软件名称或介绍…" class="!pl-9" />
      </div>
      <div class="flex items-center gap-2">
        <ListFilter class="h-4 w-4 text-[var(--muted)]" />
        <Select v-model="filter" :options="filterOptions" class="w-36" />
      </div>
    </div>

    <!-- 分类标签栏（横向滚动） -->
    <div class="no-scrollbar mb-4 flex gap-2 overflow-x-auto pb-1">
      <button
        v-for="c in CATEGORIES"
        :key="c"
        class="chip"
        :class="category === c && 'chip-active'"
        @click="category = c"
      >
        {{ c }}
      </button>
    </div>

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
  </div>
</template>
