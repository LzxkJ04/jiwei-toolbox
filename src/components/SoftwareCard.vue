<script setup lang="ts">
import { computed } from "vue";
import { Box, CheckCircle2, ChevronRight, CircleDashed, XCircle } from "lucide-vue-next";
import Checkbox from "./ui/Checkbox.vue";
import Badge from "./ui/Badge.vue";
import type { SoftwareItem } from "../lib/tauri";
import { useSoftwareStore } from "../stores/software";

const props = defineProps<{
  item: SoftwareItem;
  checked?: boolean;
}>();

const emit = defineEmits<{
  (e: "update:checked", v: boolean): void;
  (e: "detail"): void;
}>();

const store = useSoftwareStore();

const license = computed(() => store.licenseOf(props.item));
const licenseLabel = computed(
  () => ({ free: "免费", open: "开源", commercial: "商业" })[license.value]
);
const licenseTone = computed(
  () => ({ free: "success", open: "primary", commercial: "warning" })[license.value] as "success"
);

const status = computed(() => store.statusOf(props.item));
const silent = computed(() => store.silentSupported(props.item));

const statusInfo = computed(() => {
  if (status.value === "installed")
    return { text: "已安装", tone: "success" as const, icon: CheckCircle2 };
  if (!silent.value)
    return { text: "不支持静默", tone: "danger" as const, icon: XCircle };
  return { text: "未安装", tone: "gray" as const, icon: CircleDashed };
});

function onCheck(v: boolean) {
  emit("update:checked", v);
}
</script>

<template>
  <div
    class="glass-card hover-lift group cursor-pointer p-4"
    :class="checked && 'border-[var(--primary)] shadow-[0_0_0_1px_var(--primary),0_8px_24px_var(--primary-glow)]'"
    @click="emit('detail')"
  >
    <div class="flex items-start gap-3">
      <!-- 图标方块 -->
      <div
        class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl text-lg font-bold"
        :style="{ background: 'var(--primary-soft)', color: 'var(--primary)' }"
      >
        <Box v-if="!item.name" class="h-5 w-5" />
        <template v-else>{{ item.name.slice(0, 1).toUpperCase() }}</template>
      </div>

      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <span class="truncate font-semibold">{{ item.name || "未知软件" }}</span>
          <!-- 授权标签 -->
          <Badge :tone="licenseTone">{{ licenseLabel }}</Badge>
          <!-- 状态标识 -->
          <Badge :tone="statusInfo.tone" class="ml-auto">
            <component :is="statusInfo.icon" class="h-3 w-3" />
            {{ statusInfo.text }}
          </Badge>
        </div>

        <p class="mt-1 truncate text-xs text-[var(--muted)]">
          {{ item.notes || item.url || "暂无介绍" }}
        </p>

        <div class="mt-2 flex items-center gap-3 text-xs text-[var(--muted)]">
          <span>版本 {{ item.type || "exe" }}</span>
          <span v-if="item.category">· {{ item.category }}</span>
        </div>
      </div>
    </div>

    <!-- 底部：复选框 + 详情 -->
    <div class="mt-3 flex items-center justify-between border-t border-[var(--card-border)] pt-3">
      <label
        class="flex cursor-pointer items-center gap-2 text-xs text-[var(--muted)]"
        @click.stop
      >
        <Checkbox :checked="!!checked" @update:checked="onCheck" />
        勾选安装
      </label>
      <span
        class="flex items-center gap-0.5 text-xs text-[var(--muted)] opacity-0 transition-opacity group-hover:opacity-100"
      >
        详情 <ChevronRight class="h-3.5 w-3.5" />
      </span>
    </div>
  </div>
</template>
