<script setup lang="ts">
import { computed, ref } from "vue";
import { ListChecks, Search, Star, StarOff } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
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

const detailItem = ref<SoftwareItem | null>(null);
const detailOpen = ref(false);
const search = ref("");
const showClearAll = ref(false);

const favItems = computed(() => {
  const list = (store.items ?? []).filter((i) => favorites.isFavorite(i.id));
  const q = search.value.trim().toLowerCase();
  if (!q) return list;
  return list.filter((i) => `${i.name} ${i.category} ${i.description ?? ""}`.toLowerCase().includes(q));
});

/** 未安装的收藏，用于「加入队列」 */
const pendingItems = computed(() =>
  favItems.value.filter((i) => store.statusOf(i) !== "installed")
);

function openDetail(item: SoftwareItem) {
  detailItem.value = item;
  detailOpen.value = true;
}

/** 卡片上快捷取消收藏（即时移除） */
function unfav(item: SoftwareItem) {
  void favorites.toggle(item.id).then((added) => {
    toast(added ? "已加入收藏" : "已取消收藏", "success");
  });
}

/** 把收藏里还没装的软件一次性加入队列 */
function addAllToQueue() {
  const items = pendingItems.value;
  if (items.length === 0) {
    toast("收藏里的软件都已安装或已在队列中", "info");
    return;
  }
  const n = tasks.addFromSoftware(items);
  toast(n > 0 ? `已将 ${n} 个收藏软件加入任务队列` : "这些软件已在任务队列中", n > 0 ? "success" : "info");
}

/** 清空全部收藏 */
function clearAllFavorites() {
  showClearAll.value = false;
  for (const item of favItems.value) {
    void favorites.toggle(item.id);
  }
  toast("已清空收藏", "success");
}
</script>

<template>
  <div>
    <PageHeader title="我的收藏" desc="收藏常用软件，快速找到并加入安装队列">
      <template #extra>
        <Button
          variant="primary"
          size="sm"
          :disabled="pendingItems.length === 0"
          :title="`把 ${pendingItems.length} 个未安装的收藏加入队列`"
          @click="addAllToQueue"
        >
          <ListChecks class="h-3.5 w-3.5" /> 全部加入队列（{{ pendingItems.length }}）
        </Button>
        <Button variant="gray" size="sm" :disabled="favItems.length === 0" @click="showClearAll = true">
          <StarOff class="h-3.5 w-3.5" /> 清空收藏
        </Button>
      </template>
    </PageHeader>

    <div v-if="store.items === null" class="glass-card">
      <Spinner label="正在获取软件列表…" />
    </div>
    <div v-else-if="favItems.length === 0" class="glass-card">
      <EmptyState
        icon="⭐"
        text="还没有收藏任何软件"
        sub="在软件库中打开软件详情，点击「收藏」即可加入这里"
      />
    </div>
    <template v-else>
      <div class="mb-4 flex flex-wrap items-center gap-3">
        <div class="relative min-w-[220px] flex-1">
          <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]" />
          <Input v-model="search" placeholder="在收藏中搜索…" class="!pl-9" />
        </div>
        <span class="flex items-center gap-1 text-xs text-[var(--muted)]">
          <Star class="h-3.5 w-3.5" /> 共 {{ favItems.length }} 个
        </span>
      </div>

      <div v-if="favItems.length === 0" class="glass-card">
        <EmptyState text="没有匹配的收藏" sub="试试换个关键词" />
      </div>
      <div v-else class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
        <SoftwareCard
          v-for="item in favItems"
          :key="item.id"
          :item="item"
          :fav="favorites.isFavorite(item.id)"
          @detail="openDetail(item)"
          @toggle-fav="unfav(item)"
        />
      </div>
    </template>

    <ConfirmDialog
      :open="showClearAll"
      title="清空收藏"
      message="将移除全部收藏的软件，软件本身不受影响。确定继续吗？"
      confirm-text="清空收藏"
      danger
      @update:open="showClearAll = $event"
      @confirm="clearAllFavorites"
    />

    <SoftwareDetailDialog v-model:open="detailOpen" :item="detailItem" />
  </div>
</template>
