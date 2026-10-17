<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Box } from "lucide-vue-next";

/**
 * 软件图标：优先加载站点 favicon（DuckDuckGo 图标服务），
 * 失败/无域名时降级为首字母方块。
 */
const props = defineProps<{
  name: string;
  homepage?: string;
  /** 尺寸类，默认 h-11 w-11 */
  size?: string;
}>();

const failed = ref(false);
watch(
  () => props.homepage,
  () => {
    failed.value = false;
  }
);

const domain = computed(() => {
  try {
    const url = new URL(props.homepage || "");
    return url.hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
});

const faviconSrc = computed(() =>
  domain.value && !failed.value
    ? `https://icons.duckduckgo.com/ip3/${domain.value}.ico`
    : ""
);

function onError() {
  failed.value = true;
}

const boxClass = computed(() => props.size || "h-11 w-11");
</script>

<template>
  <div
    class="flex shrink-0 items-center justify-center overflow-hidden rounded-xl bg-white/90 ring-1 ring-black/5"
    :class="boxClass"
  >
    <img
      v-if="faviconSrc"
      :src="faviconSrc"
      :alt="name"
      loading="lazy"
      class="h-full w-full object-contain p-1"
      @error="onError"
    />
    <div
      v-else
      class="flex h-full w-full items-center justify-center text-lg font-bold"
      :style="{ background: 'var(--primary-soft)', color: 'var(--primary)' }"
    >
      <Box v-if="!name" class="h-5 w-5" />
      <template v-else>{{ name.slice(0, 1).toUpperCase() }}</template>
    </div>
  </div>
</template>
