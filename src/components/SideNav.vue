<script setup lang="ts">
import { computed } from "vue";
import { ChevronsLeft, ChevronsRight, Moon, Search, Sun } from "lucide-vue-next";
import { NAV_ITEMS, useRouteStore } from "../stores/route";
import { useThemeStore } from "../stores/theme";
import { cn } from "../lib/utils";

const route = useRouteStore();
const theme = useThemeStore();
const isDark = computed(() => (theme.mode === "system" ? true : theme.mode === "dark"));
function toggleTheme() {
  theme.setMode(isDark.value ? "light" : "dark");
}

/** 命令面板挂在 App 上，这里用自定义事件解耦，避免层层透传 ref */
function openPalette() {
  window.dispatchEvent(new CustomEvent("jw-open-palette"));
}
</script>

<template>
  <aside
    class="flex shrink-0 flex-col border-r border-[var(--card-border)] bg-[var(--sidebar-bg)] backdrop-blur-xl transition-[width] duration-300 ease-out"
    :class="route.collapsed ? 'w-16' : 'w-[220px]'"
  >
    <nav class="flex-1 space-y-1 overflow-y-auto overflow-x-hidden p-2.5">
      <!-- 命令面板入口 -->
      <button
        type="button"
        title="搜索页面与功能"
        class="mb-2 flex w-full cursor-pointer items-center gap-3 overflow-hidden rounded-lg border border-[var(--card-border)] py-1.5 text-xs text-[var(--muted)] transition-colors hover:border-[var(--primary)] hover:text-[var(--primary)]"
        :class="route.collapsed ? 'justify-center px-0' : 'px-3'"
        @click="openPalette"
      >
        <Search class="h-4 w-4 shrink-0" />
        <span v-if="!route.collapsed" class="truncate">搜索</span>
      </button>
      <button
        v-for="item in NAV_ITEMS"
        :key="item.id"
        :title="route.collapsed ? item.label : undefined"
        :class="
          cn(
            'relative flex w-full cursor-pointer items-center gap-3 overflow-hidden rounded-lg py-2 text-sm transition-all duration-200',
            route.collapsed ? 'justify-center px-0' : 'px-3',
            route.active === item.id
              ? 'bg-[var(--primary-soft)] font-semibold text-[var(--primary)]'
              : 'text-[var(--text)] hover:bg-[var(--btn-gray)]'
          )
        "
        @click="route.go(item.id)"
      >
        <!-- 选中项左侧指示条 -->
        <span
          aria-hidden
          :class="
            cn(
              'absolute left-0 top-1/2 w-[3px] -translate-y-1/2 rounded-full bg-[var(--primary)] transition-all duration-200',
              route.active === item.id ? 'h-5 opacity-100' : 'h-0 opacity-0'
            )
          "
        />
        <component :is="item.icon" class="h-5 w-5 shrink-0" />
        <span v-if="!route.collapsed" class="truncate">{{ item.label }}</span>
      </button>
    </nav>

    <!-- 主题切换 + 折叠开关 -->
    <button
      :title="isDark ? '切换到浅色模式' : '切换到深色模式'"
      class="mx-2.5 mb-2 flex cursor-pointer items-center justify-center gap-2 rounded-lg border border-[var(--card-border)] py-1.5 text-xs text-[var(--muted)] transition-colors hover:border-[var(--primary)] hover:text-[var(--primary)]"
      :class="route.collapsed && 'justify-center'"
      @click="toggleTheme"
    >
      <Moon v-if="isDark" class="h-4 w-4" />
      <Sun v-else class="h-4 w-4" />
      <span v-if="!route.collapsed">{{ isDark ? "深色模式" : "浅色模式" }}</span>
    </button>
    <!-- 折叠开关 -->
    <button
      :title="route.collapsed ? '展开侧栏' : '折叠侧栏'"
      class="mx-2.5 mb-3 flex cursor-pointer items-center justify-center gap-2 rounded-lg border border-[var(--card-border)] py-1.5 text-xs text-[var(--muted)] transition-colors hover:border-[var(--primary)] hover:text-[var(--primary)]"
      @click="route.toggleCollapsed()"
    >
      <component :is="route.collapsed ? ChevronsRight : ChevronsLeft" class="h-4 w-4" />
      <span v-if="!route.collapsed">折叠侧栏</span>
    </button>
  </aside>
</template>
