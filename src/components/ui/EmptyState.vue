<script setup lang="ts">
defineProps<{ icon?: string; text: string; sub?: string }>();
</script>

<template>
  <div class="flex flex-col items-center justify-center gap-2 px-6 py-14 text-center">
    <div class="flex h-14 w-14 items-center justify-center rounded-2xl bg-[var(--primary-soft)] text-[var(--primary)]">
      <svg viewBox="0 0 24 24" class="h-7 w-7" fill="none" stroke="currentColor" stroke-width="1.6">
        <path stroke-linecap="round" stroke-linejoin="round" d="M4 8.5 12 4l8 4.5v7L12 20l-8-4.5v-7z" />
        <path stroke-linecap="round" d="M12 12.2 20 8.2M12 12.2V20M12 12.2 4 8.2" />
      </svg>
    </div>
    <p class="text-sm font-medium text-[var(--text)]">{{ text }}</p>
    <p v-if="sub" class="max-w-sm text-xs text-[var(--muted)]">{{ sub }}</p>
  </div>
</template>
