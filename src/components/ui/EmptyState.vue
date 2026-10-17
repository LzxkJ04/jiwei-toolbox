<script setup lang="ts">
defineProps<{ icon?: string; text: string; sub?: string }>();
</script>

<template>
  <div class="flex flex-col items-center justify-center gap-2 px-6 py-14 text-center">
    <div class="text-4xl">{{ icon ?? "📦" }}</div>
    <p class="text-sm font-medium text-[var(--text)]">{{ text }}</p>
    <p v-if="sub" class="max-w-sm text-xs text-[var(--muted)]">{{ sub }}</p>
  </div>
</template>
