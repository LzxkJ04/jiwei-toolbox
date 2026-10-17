<script setup lang="ts">
import {
  SelectRoot, SelectTrigger, SelectValue, SelectPortal, SelectContent, SelectViewport,
  SelectItem, SelectItemText, SelectItemIndicator,
} from "radix-vue";
import { Check, ChevronDown } from "lucide-vue-next";
import { cn } from "../../lib/utils";

export interface SelectOption {
  value: string;
  label: string;
}

const props = defineProps<{
  options: SelectOption[];
  modelValue: string;
  placeholder?: string;
  class?: string;
  contentClass?: string;
}>();

const emit = defineEmits<{ (e: "update:modelValue", v: string): void }>();
</script>

<template>
  <SelectRoot :model-value="props.modelValue" @update:model-value="emit('update:modelValue', String($event))">
    <SelectTrigger
      :class="
        cn(
          'input-base inline-flex cursor-pointer items-center justify-between gap-2 !py-2 text-sm',
          props.class
        )
      "
    >
      <SelectValue :placeholder="props.placeholder ?? '请选择'" />
      <ChevronDown class="h-4 w-4 shrink-0 text-[var(--muted)]" />
    </SelectTrigger>
    <SelectPortal>
      <SelectContent
        position="popper"
        :side-offset="6"
        :class="
          cn(
            'glass-card z-50 max-h-72 min-w-[10rem] overflow-hidden rounded-xl !bg-[var(--card-solid)]/95 p-1 backdrop-blur-xl',
            props.contentClass
          )
        "
      >
        <SelectViewport>
          <SelectItem
            v-for="opt in props.options"
            :key="opt.value"
            :value="opt.value"
            class="relative flex cursor-pointer select-none items-center rounded-lg py-1.5 pl-8 pr-3 text-sm outline-none transition-colors data-[highlighted]:bg-[var(--primary-soft)] data-[highlighted]:text-[var(--primary)]"
          >
            <SelectItemText>{{ opt.label }}</SelectItemText>
            <SelectItemIndicator class="absolute left-2 inline-flex items-center">
              <Check class="h-3.5 w-3.5" />
            </SelectItemIndicator>
          </SelectItem>
        </SelectViewport>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>
