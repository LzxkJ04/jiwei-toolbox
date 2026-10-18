<script setup lang="ts">
import { onUnmounted, onMounted, ref, computed } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Pin, PinOff, Square, X, Sun, Moon } from "lucide-vue-next";
import { useThemeStore } from "../stores/theme";

const theme = useThemeStore();
const isDark = computed(() => (theme.mode === "system" ? true : theme.mode === "dark"));
function toggleTheme() {
  theme.setMode(isDark.value ? "light" : "dark");
}

const appWindow = getCurrentWindow();
const maximized = ref(false);
const pinned = ref(false);

let unlisten: (() => void) | null = null;
onMounted(async () => {
  try {
    maximized.value = await appWindow.isMaximized();
    const un = await appWindow.onResized(async () => {
      maximized.value = await appWindow.isMaximized();
    });
    unlisten = un;
  } catch {
    /* 浏览器调试环境忽略 */
  }
});
onUnmounted(() => unlisten?.());

async function togglePin() {
  pinned.value = !pinned.value;
  try {
    await appWindow.setAlwaysOnTop(pinned.value);
  } catch {
    /* ignore */
  }
}
</script>

<template>
  <!-- 自制标题栏：无边框窗口拖拽区 -->
  <header
    data-tauri-drag-region
    class="flex h-10 shrink-0 select-none items-center border-b border-[var(--card-border)] bg-[var(--sidebar-bg)] pl-3 pr-1 backdrop-blur-xl"
  >
    <!-- 左：logo + 应用名 -->
    <div data-tauri-drag-region class="flex items-center gap-2">
      <div
        class="flex h-6 w-6 items-center justify-center rounded-md bg-gradient-to-b from-[var(--primary-hov)] to-[var(--primary)] text-xs font-bold text-white shadow-[0_0_10px_var(--primary-glow)]"
      >
        极
      </div>
      <span data-tauri-drag-region class="text-sm font-semibold tracking-wide">极维工具箱</span>
    </div>

    <!-- 中：空白拖拽区 -->
    <div data-tauri-drag-region class="h-full flex-1" />

    <!-- 右：窗口控制 -->
    <div class="flex items-center gap-0.5">
      <button
        :title="pinned ? '取消置顶' : '窗口置顶'"
        class="flex h-8 w-9 cursor-pointer items-center justify-center rounded-md transition-colors"
        :class="pinned ? 'bg-[var(--primary-soft)] text-[var(--primary)]' : 'text-[var(--muted)] hover:bg-[var(--btn-gray)] hover:text-[var(--text)]'"
        @click="togglePin"
      >
        <component :is="pinned ? Pin : PinOff" class="h-4 w-4" />
      </button>
      <button
        title="最小化"
        class="flex h-8 w-9 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--btn-gray)] hover:text-[var(--text)]"
        @click="appWindow.minimize()"
      >
        <Minus class="h-4 w-4" />
      </button>
      <!-- 深浅主题切换 -->
      <button
        :title="isDark ? '切换到浅色模式' : '切换到深色模式'"
        class="flex h-8 w-9 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--btn-gray)] hover:text-[var(--text)]"
        @click="toggleTheme"
      >
        <Sun v-if="isDark" class="h-4 w-4" />
        <Moon v-else class="h-4 w-4" />
      </button>
      <button
        :title="maximized ? '还原' : '最大化'"
        class="flex h-8 w-9 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--btn-gray)] hover:text-[var(--text)]"
        @click="appWindow.toggleMaximize()"
      >
        <component :is="maximized ? Copy : Square" class="h-3.5 w-3.5" />
      </button>
      <button
        title="关闭"
        class="flex h-8 w-9 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[#e5484d] hover:text-white"
        @click="appWindow.close()"
      >
        <X class="h-4 w-4" />
      </button>
    </div>
  </header>
</template>
