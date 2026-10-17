<script setup lang="ts">
import { cn } from "../../lib/utils";

const props = defineProps<{
  value?: number; // 0-100
  class?: string;
  /** 进度条配色：primary（默认）/ success / warning / danger */
  tone?: "primary" | "success" | "warning" | "danger";
}>();

const tones: Record<string, string> = {
  primary: "linear-gradient(90deg, var(--primary-hov), var(--primary))",
  success: "linear-gradient(90deg, #30a46c, #4ccb8b)",
  warning: "linear-gradient(90deg, #f59e0b, #fbbf24)",
  danger: "linear-gradient(90deg, #e5484d, #ff6b6f)",
};
</script>

<template>
  <div :class="cn('h-2 w-full overflow-hidden rounded-full bg-[var(--btn-gray)]', props.class)">
    <div
      class="h-full rounded-full transition-[width] duration-200"
      :style="{ width: `${Math.min(100, Math.max(0, props.value ?? 0))}%`, background: tones[props.tone ?? 'primary'] }"
    />
  </div>
</template>
