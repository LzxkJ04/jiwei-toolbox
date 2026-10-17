<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { Box } from "lucide-vue-next";
import { appIcon, type SoftwareItem } from "../lib/tauri";

/**
 * 软件图标：后端 app_icon（磁盘缓存 → 在线拉取 → 离线可用）返回 data URL；
 * 失败/无域名降级首字母方块。
 */
const props = defineProps<{
  item: Pick<SoftwareItem, "name" | "homepage">;
  /** 尺寸类，默认 h-11 w-11 */
  size?: string;
}>();

const src = ref("");
const failed = ref(false);

async function load() {
  failed.value = false;
  src.value = "";
  if (!props.item.homepage) {
    failed.value = true;
    return;
  }
  const data = await appIcon(props.item.homepage);
  if (data) src.value = data;
  else failed.value = true;
}

onMounted(load);
watch(() => props.item.homepage, load);

const boxClass = () => props.size || "h-11 w-11";
</script>

<template>
  <div
    class="flex shrink-0 items-center justify-center overflow-hidden rounded-xl bg-white/90 ring-1 ring-black/5"
    :class="boxClass()"
  >
    <img v-if="src" :src="src" :alt="item.name" class="h-full w-full object-contain p-1" />
    <div
      v-else
      class="flex h-full w-full items-center justify-center text-lg font-bold"
      :style="{
        background: failed ? 'var(--primary-soft)' : 'var(--border)',
        color: 'var(--primary)',
        animation: !failed ? 'pulse 1.2s ease-in-out infinite' : undefined,
      }"
    >
      <Box v-if="!item.name" class="h-5 w-5" />
      <template v-else>{{ item.name.slice(0, 1).toUpperCase() }}</template>
    </div>
  </div>
</template>
