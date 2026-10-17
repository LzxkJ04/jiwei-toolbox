<script setup lang="ts">
import { cn } from "../../lib/utils";

const props = defineProps<{ class?: string; hover?: boolean }>();
</script>

<template>
  <section :class="cn('glass-card', props.hover && 'hover-lift', props.class)">
    <slot />
  </section>
</template>
