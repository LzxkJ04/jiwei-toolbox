<script setup lang="ts">
import { cn } from "../../lib/utils";

const props = defineProps<{
  tone?: "primary" | "success" | "warning" | "danger" | "gray";
  class?: string;
}>();

const tones: Record<string, string> = {
  primary: "bg-[var(--primary-soft)] text-[var(--primary)]",
  success: "bg-[rgba(48,164,108,.14)] text-[#30a46c]",
  warning: "bg-[rgba(245,158,11,.14)] text-[#f59e0b]",
  danger: "bg-[rgba(229,72,77,.14)] text-[#e5484d]",
  gray: "bg-[var(--btn-gray)] text-[var(--muted)]",
};
</script>

<template>
  <span
    :class="
      cn('inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium', tones[props.tone ?? 'gray'], props.class)
    "
  >
    <slot />
  </span>
</template>
