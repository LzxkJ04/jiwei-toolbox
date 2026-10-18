<script setup lang="ts">
import {
  DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogClose,
} from "radix-vue";
import { X } from "lucide-vue-next";
import { cn } from "../../lib/utils";

const props = defineProps<{
  open: boolean;
  title?: string;
  class?: string;
  /** 内容区宽度，默认 max-w-lg */
}>();

const emit = defineEmits<{ (e: "update:open", v: boolean): void }>();
</script>

<template>
  <DialogRoot :open="props.open" @update:open="emit('update:open', $event)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/55 backdrop-blur-sm" />
      <DialogContent
        :class="
          cn(
            'dialog-panel-anim glass-card fixed left-1/2 top-1/2 z-50 max-h-[85vh] w-[calc(100vw-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 overflow-y-auto p-0 focus:outline-none',
            props.class
          )
        "
      >
        <div
          v-if="props.title !== undefined"
          class="sticky top-0 z-10 flex items-center justify-between gap-3 border-b border-[var(--card-border)] bg-[var(--card-solid)]/90 px-5 py-3.5 backdrop-blur"
        >
          <DialogTitle class="min-w-0 flex-1 truncate font-semibold">{{ props.title }}</DialogTitle>
          <DialogClose
            class="flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--btn-gray)] hover:text-[var(--text)]"
          >
            <X class="h-4 w-4" />
          </DialogClose>
        </div>
        <div class="px-5 py-4"><slot /></div>
        <div v-if="$slots.footer" class="border-t border-[var(--card-border)] px-5 py-3.5">
          <slot name="footer" />
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
