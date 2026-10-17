<script setup lang="ts">
import { CheckboxRoot, CheckboxIndicator } from "radix-vue";
import { Check } from "lucide-vue-next";
import { cn } from "../../lib/utils";

const props = defineProps<{ class?: string; checked?: boolean; disabled?: boolean }>();

const emit = defineEmits<{ (e: "update:checked", v: boolean): void }>();
</script>

<template>
  <CheckboxRoot
    :checked="props.checked"
    :disabled="props.disabled"
    :class="
      cn(
        'flex h-[18px] w-[18px] shrink-0 cursor-pointer items-center justify-center rounded border transition-colors',
        props.checked
          ? 'border-transparent bg-[var(--primary)] text-white shadow-[0_1px_6px_var(--primary-glow)]'
          : 'border-[var(--card-border)] bg-[var(--card-soft)] hover:border-[var(--primary)]',
        props.class
      )
    "
    @update:checked="emit('update:checked', $event)"
  >
    <CheckboxIndicator>
      <Check class="h-3 w-3" :stroke-width="3" />
    </CheckboxIndicator>
  </CheckboxRoot>
</template>
