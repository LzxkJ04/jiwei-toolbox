<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { AlertTriangle, CircleCheck, CircleX, Info } from "lucide-vue-next";
import { subscribeToast, type ToastItem } from "../../lib/toast";

const items = ref<ToastItem[]>([]);
const unsub = subscribeToast((t) => {
  items.value.push(t);
  window.setTimeout(() => {
    const idx = items.value.findIndex((x) => x.id === t.id);
    if (idx >= 0) items.value.splice(idx, 1);
  }, 3200);
});
onUnmounted(() => unsub());

const iconOf = (kind: string) =>
  kind === "success"
    ? CircleCheck
    : kind === "error"
      ? CircleX
      : kind === "warning"
        ? AlertTriangle
        : Info;
const toneOf = (kind: string) =>
  kind === "success"
    ? "text-[#30a46c]"
    : kind === "error"
      ? "text-[#e5484d]"
      : kind === "warning"
        ? "text-[#f5a623]"
        : "text-[var(--primary)]";
</script>

<template>
  <div class="pointer-events-none fixed left-1/2 top-12 z-[100] flex w-max max-w-[90vw] -translate-x-1/2 flex-col items-center gap-2">
    <TransitionGroup name="toast">
      <div
        v-for="t in items"
        :key="t.id"
        class="toast-item glass-card pointer-events-auto flex items-center gap-2.5 !rounded-xl px-4 py-2.5 text-sm shadow-glass"
      >
        <component :is="iconOf(t.kind)" :class="['h-4 w-4 shrink-0', toneOf(t.kind)]" />
        <span class="whitespace-pre-wrap">{{ t.msg }}</span>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-leave-active,
.toast-move {
  transition: all 0.2s ease;
}
.toast-enter-active {
  transition: all 0.25s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}
</style>
