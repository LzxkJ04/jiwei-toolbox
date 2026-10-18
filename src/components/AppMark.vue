<script setup lang="ts">
import { useId } from "vue";

withDefaults(defineProps<{ size?: number }>(), { size: 24 });
const gid = useId();
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 64 64"
    fill="none"
    aria-hidden="true"
    class="shrink-0"
  >
    <defs>
      <linearGradient :id="gid" x1="8" y1="4" x2="58" y2="62" gradientUnits="userSpaceOnUse">
        <stop stop-color="#C4B5FD" />
        <stop offset="1" stop-color="#6D28D9" />
      </linearGradient>
    </defs>
    <rect width="64" height="64" rx="16" :fill="`url(#${gid})`" />
    <ellipse cx="32" cy="8" rx="22" ry="14" fill="#fff" fill-opacity=".22" />
    <rect x="13" y="26" width="38" height="24" rx="6" fill="#fff" />
    <path
      d="M24 28v-3.2c0-4.4 3.6-8 8-8s8 3.6 8 8V28"
      stroke="#fff"
      stroke-width="4.2"
      stroke-linecap="round"
    />
    <rect x="28.5" y="33" width="7" height="10" rx="2" fill="#6D28D9" />
    <path
      d="M48 14.2l1.1 2.7 2.7 1.1-2.7 1.1-1.1 2.7-1.1-2.7-2.7-1.1 2.7-1.1 1.1-2.7z"
      fill="#fff"
    />
  </svg>
</template>
