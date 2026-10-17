<script setup lang="ts">
import { computed, ref } from "vue";
import PageHeader from "../components/ui/PageHeader.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import SoftwareCard from "../components/SoftwareCard.vue";
import SoftwareDetailDialog from "../components/SoftwareDetailDialog.vue";
import type { SoftwareItem } from "../lib/tauri";
import { useSoftwareStore } from "../stores/software";
import { useFavoritesStore } from "../stores/favorites";

const store = useSoftwareStore();
const favorites = useFavoritesStore();

const detailItem = ref<SoftwareItem | null>(null);
const detailOpen = ref(false);

const favItems = computed(() =>
  (store.items ?? []).filter((i) => favorites.isFavorite(i.id))
);

function openDetail(item: SoftwareItem) {
  detailItem.value = item;
  detailOpen.value = true;
}
</script>

<template>
  <div>
    <PageHeader title="我的收藏" desc="收藏常用软件，快速找到并加入安装队列" />

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
    <div v-else class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
      <SoftwareCard
        v-for="item in favItems"
        :key="item.id"
        :item="item"
        @detail="openDetail(item)"
      />
    </div>

    <SoftwareDetailDialog v-model:open="detailOpen" :item="detailItem" />
  </div>
</template>
