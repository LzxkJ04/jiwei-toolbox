<script setup lang="ts">
defineProps<{ title: string; desc?: string }>();
</script>

<template>
  <div class="mb-5 flex flex-wrap items-end justify-between gap-3">
    <div>
      <h2 class="text-xl font-bold text-[var(--text-strong)]">{{ title }}</h2>
      <p v-if="desc" class="mt-1 text-sm text-[var(--muted)]">{{ desc }}</p>
    </div>
    <div class="flex items-center gap-2"><slot name="extra" /></div>
  </div>
</template>
