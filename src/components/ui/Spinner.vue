<script setup lang="ts">
defineProps<{ label?: string }>();
</script>

<template>
  <div class="flex items-center justify-center gap-2.5 py-10">
    <span class="spinner" />
    <span v-if="label" class="text-sm text-[var(--muted)]">{{ label }}</span>
  </div>
</template>
