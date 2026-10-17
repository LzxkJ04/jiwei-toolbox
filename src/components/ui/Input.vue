<script setup lang="ts">
import { cn } from "../../lib/utils";

const props = defineProps<{
  class?: string;
  modelValue?: string;
  placeholder?: string;
  type?: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{ (e: "update:modelValue", v: string): void }>();

function onInput(e: Event) {
  emit("update:modelValue", (e.target as HTMLInputElement).value);
}
</script>

<template>
  <input
    :type="props.type ?? 'text'"
    :value="props.modelValue"
    :placeholder="props.placeholder"
    :disabled="props.disabled"
    :class="cn('input-base', props.class)"
    @input="onInput"
  />
</template>
