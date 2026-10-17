<script setup lang="ts">
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const buttonVariants = cva("btn", {
  variants: {
    variant: {
      primary: "btn-primary",
      gray: "btn-gray",
      danger: "btn-danger",
      ghost: "hover:bg-[var(--btn-gray)]",
    },
    size: {
      default: "",
      sm: "!px-3 !py-1.5 text-xs",
      icon: "!px-2 !py-2",
    },
  },
  defaultVariants: { variant: "gray", size: "default" },
});

const props = defineProps<{
  variant?: VariantProps<typeof buttonVariants>["variant"];
  size?: VariantProps<typeof buttonVariants>["size"];
  class?: string;
  disabled?: boolean;
  type?: "button" | "submit";
  title?: string;
}>();
</script>

<template>
  <button
    :type="props.type ?? 'button'"
    :title="props.title"
    :disabled="props.disabled"
    :class="cn(buttonVariants({ variant, size }), props.class)"
  >
    <slot />
  </button>
</template>
