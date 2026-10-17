<script setup lang="ts">
import Dialog from "./Dialog.vue";
import Button from "./Button.vue";

const props = defineProps<{
  open: boolean;
  title: string;
  message: string;
  confirmText?: string;
  /** 取消按钮文案，默认"取消" */
  cancelText?: string;
  danger?: boolean;
}>();

const emit = defineEmits<{
  (e: "update:open", v: boolean): void;
  (e: "confirm"): void;
}>();

function onConfirm() {
  emit("confirm");
  emit("update:open", false);
}
</script>

<template>
  <Dialog :open="props.open" :title="props.title" @update:open="emit('update:open', $event)">
    <p class="text-sm leading-relaxed text-[var(--text)]">{{ props.message }}</p>
    <template #footer>
      <div class="flex justify-end gap-3">
        <Button variant="gray" @click="emit('update:open', false)">{{ props.cancelText ?? "取消" }}</Button>
        <Button :variant="props.danger ? 'danger' : 'primary'" @click="onConfirm">
          {{ props.confirmText ?? "确定" }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
