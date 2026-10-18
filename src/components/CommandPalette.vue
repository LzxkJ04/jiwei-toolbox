<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { CornerDownLeft, Search } from "lucide-vue-next";
import { NAV_ITEMS, useRouteStore, type PageId } from "../stores/route";
import { useThemeStore } from "../stores/theme";
import { ACCENTS, type Accent } from "../stores/theme";

/** 全局命令面板：Ctrl+K 打开，可跳页、切主题、切主色 */

interface Cmd {
  id: string;
  label: string;
  hint?: string;
  group: string;
  run: () => void;
}

const open = ref(false);
const keyword = ref("");
const cursor = ref(0);
const inputEl = ref<HTMLInputElement | null>(null);

const route = useRouteStore();
const theme = useThemeStore();

const commands = computed<Cmd[]>(() => {
  const pages: Cmd[] = NAV_ITEMS.map((n) => ({
    id: `go:${n.id}`,
    label: n.label,
    hint: "跳转",
    group: "页面",
    run: () => route.go(n.id as PageId),
  }));
  const tones: Cmd[] = [
    {
      id: "theme:toggle",
      label: theme.resolved === "dark" ? "切换到浅色模式" : "切换到深色模式",
      hint: "外观",
      group: "外观",
      run: () => theme.setMode(theme.resolved === "dark" ? "light" : "dark"),
    },
    ...ACCENTS.map((a) => ({
      id: `accent:${a.id}`,
      label: `主色改为 ${a.label}`,
      hint: a.color,
      group: "外观",
      run: () => theme.setAccent(a.id as Accent),
    })),
    {
      id: "side:toggle",
      label: route.collapsed ? "展开侧栏" : "折叠侧栏",
      hint: "布局",
      group: "外观",
      run: () => route.toggleCollapsed(),
    },
  ];
  return [...pages, ...tones];
});

const shown = computed(() => {
  const kw = keyword.value.trim().toLowerCase();
  const list = kw
    ? commands.value.filter((c) => `${c.label} ${c.group}`.toLowerCase().includes(kw))
    : commands.value;
  return list;
});

watch(shown, () => {
  cursor.value = 0;
});

function openPanel() {
  open.value = true;
  keyword.value = "";
  cursor.value = 0;
  void nextTick(() => inputEl.value?.focus());
}

function closePanel() {
  open.value = false;
}

function execute(cmd?: Cmd) {
  const target = cmd ?? shown.value[cursor.value];
  if (!target) return;
  closePanel();
  target.run();
}

function onKey(e: KeyboardEvent) {
  if (!open.value) return;
  if (e.key === "Escape") {
    e.preventDefault();
    closePanel();
  }
}

onMounted(() => {
  document.addEventListener("keydown", onKey);
  window.addEventListener("jw-open-palette", openPanel);
});

onUnmounted(() => {
  document.removeEventListener("keydown", onKey);
  window.removeEventListener("jw-open-palette", openPanel);
});

/** 供外部（如标题栏按钮）触发 */
defineExpose({ openPanel });
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 z-[200] flex items-start justify-center bg-black/50 px-4 pt-[12vh] backdrop-blur-sm"
    @click.self="closePanel"
  >
    <div class="glass-card w-full max-w-xl overflow-hidden !bg-[var(--card-solid)]/97 p-0">
      <div class="flex items-center gap-2 border-b border-[var(--card-border)] px-4 py-3">
        <Search class="h-4 w-4 shrink-0 text-[var(--muted)]" />
        <input
          ref="inputEl"
          v-model="keyword"
          class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-[var(--muted)]"
          placeholder="搜索页面或命令…"
          @keydown="onKey"
        />
        <kbd class="shrink-0 rounded border border-[var(--card-border)] px-1.5 py-0.5 text-[10px] text-[var(--muted)]">Esc</kbd>
      </div>
      <div class="max-h-[52vh] overflow-y-auto p-1.5">
        <p v-if="!shown.length" class="px-3 py-8 text-center text-sm text-[var(--muted)]">没有匹配的命令</p>
        <button
          v-for="(c, i) in shown"
          :key="c.id"
          type="button"
          :data-idx="i"
          class="flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors"
          :class="i === cursor ? 'bg-[var(--primary-soft)] text-[var(--primary)]' : 'text-[var(--text)] hover:bg-[var(--btn-gray)]'"
          @mouseenter="cursor = i"
          @click="execute(c)"
        >
          <span class="min-w-0 truncate">{{ c.label }}</span>
          <span class="flex shrink-0 items-center gap-2 text-[10px] text-[var(--muted)]">
            {{ c.group }}
            <CornerDownLeft v-if="i === cursor" class="h-3 w-3" />
          </span>
        </button>
      </div>
      <div class="flex items-center justify-between border-t border-[var(--card-border)] px-4 py-2 text-[10px] text-[var(--muted)]">
        <span>点击条目执行</span>
        <span>按 Esc 关闭</span>
      </div>
    </div>
  </div>
</template>
