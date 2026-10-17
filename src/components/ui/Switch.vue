<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from "radix-vue";
import { cn } from "../../lib/utils";

const props = defineProps<{ checked?: boolean; disabled?: boolean; class?: string }>();

const emit = defineEmits<{ (e: "update:checked", v: boolean): void }>();
</script>

<template>
  <SwitchRoot
    :checked="props.checked"
    :disabled="props.disabled"
    :class="
      cn(
        'relative h-6 w-11 shrink-0 cursor-pointer rounded-full border border-[var(--card-border)] transition-colors',
        props.checked ? 'bg-[var(--primary)]' : 'bg-[var(--btn-gray)]',
        props.class
      )
    "
    @update:checked="emit('update:checked', $event)"
  >
    <SwitchThumb
      :class="
        cn(
          'block h-4 w-4 translate-x-1 rounded-full bg-white shadow transition-transform',
          props.checked && 'translate-x-6'
        )
      "
    />
  </SwitchRoot>
</template>
