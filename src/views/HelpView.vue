<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import {
  ArrowUp,
  BookOpen,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Copy,
  Search,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Badge from "../components/ui/Badge.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import { HELP_CHAPTERS, type HelpBlock, type HelpSection } from "../lib/helpContent";
import { copyText } from "../lib/utils";
import { toast } from "../lib/toast";

/* ============ 渲染模型：把联合类型压平成模板好用的结构 ============ */
interface RenderBlock {
  kind: "p" | "list" | "kv" | "note";
  text: string;
  title: string;
  items: string[];
  rows: { k: string; v: string }[];
  icon: string;
  boxClass: string;
}

const NOTE_STYLE: Record<"info" | "warn" | "danger", { icon: string; boxClass: string }> = {
  info: { icon: "💡", boxClass: "border-l-[#3b82f6] bg-[rgba(59,130,246,0.09)]" },
  warn: { icon: "⚠", boxClass: "border-l-[#f59e0b] bg-[rgba(245,158,11,0.10)]" },
  danger: { icon: "🛑", boxClass: "border-l-[#e5484d] bg-[rgba(229,72,77,0.10)]" },
};

function toRender(b: HelpBlock): RenderBlock {
  const base: RenderBlock = { kind: "p", text: "", title: "", items: [], rows: [], icon: "", boxClass: "" };
  if (b.type === "p") return { ...base, kind: "p", text: b.text };
  if (b.type === "list") return { ...base, kind: "list", items: b.items };
  if (b.type === "kv") return { ...base, kind: "kv", rows: b.rows };
  const style = NOTE_STYLE[b.type];
  return { ...base, kind: "note", icon: style.icon, boxClass: style.boxClass, title: b.title ?? "", text: b.text };
}

/* ============ 状态 ============ */
const keyword = ref("");
const expanded = ref<Set<string>>(new Set(HELP_CHAPTERS.map((c) => c.id)));
const activeChapterId = ref(HELP_CHAPTERS[0].id);
const activeId = ref(HELP_CHAPTERS[0].sections[0]?.id ?? "");
const showTop = ref(false);
const scroller = ref<HTMLElement | null>(null);
/** 平滑滚动期间锁住高亮，避免中途的 scroll 事件把选中项改回去 */
let lockUntil = 0;

const currentChapter = computed(
  () => HELP_CHAPTERS.find((c) => c.id === activeChapterId.value) ?? HELP_CHAPTERS[0],
);

const currentIndex = computed(() => HELP_CHAPTERS.findIndex((c) => c.id === currentChapter.value.id));

const renderSections = computed(() =>
  currentChapter.value.sections.map((s) => ({
    id: s.id,
    title: s.title,
    blocks: s.blocks.map(toRender),
  })),
);

/* ============ 搜索：标题 + 小节标题 + 全部 block 文字 ============ */
function sectionText(s: HelpSection): string {
  const parts: string[] = [s.title];
  for (const b of s.blocks) {
    if (b.type === "p") parts.push(b.text);
    else if (b.type === "list") parts.push(...b.items);
    else if (b.type === "kv") b.rows.forEach((r) => parts.push(r.k, r.v));
    else parts.push(b.title ?? "", b.text);
  }
  return parts.join(" ").toLowerCase();
}

const kw = computed(() => keyword.value.trim().toLowerCase());

/** 过滤后的目录树：章节标题命中则保留全部小节，否则只保留命中的小节 */
const visibleTree = computed(() => {
  if (!kw.value) return HELP_CHAPTERS.map((c) => ({ chapter: c, sections: c.sections }));
  return HELP_CHAPTERS.map((c) => {
    const titleHit = c.title.toLowerCase().includes(kw.value);
    const sections = titleHit ? c.sections : c.sections.filter((s) => sectionText(s).includes(kw.value));
    return { chapter: c, sections };
  }).filter((x) => x.sections.length > 0);
});

const hitCount = computed(() => visibleTree.value.reduce((n, x) => n + x.sections.length, 0));

/** 搜索时命中章节自动展开（只增不减，用户仍可手动折叠） */
function autoExpandHits() {
  if (!kw.value) return;
  const next = new Set(expanded.value);
  for (const x of visibleTree.value) next.add(x.chapter.id);
  expanded.value = next;
}

function isOpen(id: string): boolean {
  return expanded.value.has(id);
}

function toggleChapter(id: string) {
  const next = new Set(expanded.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  expanded.value = next;
}

/* ============ 滚动与定位 ============ */
/** 用容器内的相对偏移定位，避免 scrollIntoView 带动整页滚动 */
function scrollToSection(id: string) {
  const box = scroller.value;
  const el = document.getElementById(id);
  if (!box || !el) return;
  const offset = el.getBoundingClientRect().top - box.getBoundingClientRect().top + box.scrollTop;
  box.scrollTo({ top: Math.max(0, offset - 16), behavior: "smooth" });
  activeId.value = id;
  lockUntil = Date.now() + 700;
}

function onScroll() {
  const box = scroller.value;
  if (!box) return;
  showTop.value = box.scrollTop > 400;
  if (Date.now() < lockUntil) return;

  const sections = currentChapter.value.sections;
  if (sections.length === 0) return;
  // 滚到底部时直接高亮最后一节，避免短小节永远够不到顶部判定线
  if (box.scrollTop + box.clientHeight >= box.scrollHeight - 4) {
    activeId.value = sections[sections.length - 1].id;
    return;
  }
  let current = sections[0].id;
  for (const s of sections) {
    const el = document.getElementById(s.id);
    if (!el) continue;
    const top = el.getBoundingClientRect().top - box.getBoundingClientRect().top;
    if (top <= 40) current = s.id;
    else break;
  }
  activeId.value = current;
}

/** 点击目录项：跨章节时先切章节再定位 */
async function selectSection(chapterId: string, sectionId: string) {
  if (chapterId !== activeChapterId.value) {
    activeChapterId.value = chapterId;
    await nextTick();
    if (scroller.value) scroller.value.scrollTop = 0;
    await nextTick();
  }
  scrollToSection(sectionId);
}

/** 切换章节：回到顶部，高亮该章第一节 */
async function gotoChapter(index: number) {
  const chapter = HELP_CHAPTERS[index];
  if (!chapter || chapter.sections.length === 0) return;
  // 翻章时确保目标章节在目录中处于展开状态，便于看到当前选中项
  if (!expanded.value.has(chapter.id)) {
    const next = new Set(expanded.value);
    next.add(chapter.id);
    expanded.value = next;
  }
  activeChapterId.value = chapter.id;
  activeId.value = chapter.sections[0].id;
  await nextTick();
  if (scroller.value) scroller.value.scrollTop = 0;
  showTop.value = false;
  lockUntil = Date.now() + 200;
}

function backToTop() {
  const box = scroller.value;
  if (!box) return;
  box.scrollTo({ top: 0, behavior: "smooth" });
  activeId.value = currentChapter.value.sections[0]?.id ?? "";
  lockUntil = Date.now() + 500;
}

/* ============ 导出为 Markdown 纯文本 ============ */
function buildMarkdown(): string {
  const out: string[] = ["极维工具箱 · 使用帮助", ""];
  for (const chapter of HELP_CHAPTERS) {
    out.push(`# ${chapter.title}`, "");
    for (const section of chapter.sections) {
      out.push(`## ${section.title}`, "");
      for (const b of section.blocks) {
        if (b.type === "p") {
          out.push(b.text, "");
        } else if (b.type === "list") {
          b.items.forEach((i) => out.push(`- ${i}`));
          out.push("");
        } else if (b.type === "kv") {
          b.rows.forEach((r) => out.push(`- ${r.k}：${r.v}`));
          out.push("");
        } else {
          const mark = b.type === "info" ? "💡" : b.type === "warn" ? "⚠" : "🛑";
          const head = b.title ? `${b.title}：` : "";
          out.push(`> ${mark} ${head}${b.text}`, "");
        }
      }
    }
  }
  return out.join("\n").trimEnd() + "\n";
}

async function exportDoc() {
  const ok = await copyText(buildMarkdown());
  if (ok) toast("帮助文档已复制到剪贴板", "success");
  else toast("复制失败，请检查剪贴板权限", "error");
}

// 搜索词变化时自动展开命中的章节，保证结果可见
watch(kw, autoExpandHits);
</script>

<template>
  <div>
    <PageHeader title="帮助文档" desc="从首次启动到故障排查，遇到问题先在这里找答案">
      <template #extra>
        <Button variant="gray" size="sm" @click="exportDoc">
          <Copy class="h-3.5 w-3.5" /> 导出文档
        </Button>
      </template>
    </PageHeader>

    <div class="flex flex-col gap-4 lg:flex-row lg:items-start">
      <!-- ==================== 左侧目录 ==================== -->
      <aside class="w-full shrink-0 lg:w-60">
        <Card class="flex max-h-[46vh] flex-col overflow-hidden p-0 lg:h-[calc(100vh_-_13rem)] lg:max-h-none">
          <!-- 搜索 -->
          <div class="border-b border-[var(--card-border)] p-3">
            <div class="relative">
              <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-[var(--muted)]" />
              <Input v-model="keyword" placeholder="搜索标题或正文" class="pl-8" />
            </div>
            <p v-if="kw" class="mt-2 text-xs text-[var(--muted)]">
              命中 {{ hitCount }} 个小节
            </p>
          </div>

          <!-- 目录树 -->
          <div class="min-h-0 flex-1 overflow-y-auto p-2">
            <div v-if="visibleTree.length === 0" class="px-2 py-6 text-center text-xs leading-relaxed text-[var(--muted)]">
              没有匹配的内容<br />换个关键词试试
            </div>

            <div v-for="group in visibleTree" :key="group.chapter.id" class="mb-1">
              <button
                type="button"
                class="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-sm font-semibold text-[var(--text-strong)] transition-colors hover:bg-[var(--btn-gray)]"
                @click="toggleChapter(group.chapter.id)"
              >
                <ChevronDown v-if="isOpen(group.chapter.id)" class="h-3.5 w-3.5 shrink-0 text-[var(--muted)]" />
                <ChevronRight v-else class="h-3.5 w-3.5 shrink-0 text-[var(--muted)]" />
                <span class="min-w-0 flex-1 truncate">{{ group.chapter.title }}</span>
                <Badge tone="gray">{{ group.sections.length }}</Badge>
              </button>

              <div v-show="isOpen(group.chapter.id)" class="mt-0.5 space-y-0.5 pl-3">
                <button
                  v-for="section in group.sections"
                  :key="section.id"
                  type="button"
                  class="block w-full truncate rounded-lg px-2 py-1.5 text-left text-xs leading-relaxed transition-colors"
                  :class="
                    section.id === activeId
                      ? 'bg-[var(--primary-soft)] font-medium text-[var(--primary)]'
                      : 'text-[var(--muted)] hover:bg-[var(--btn-gray)] hover:text-[var(--text)]'
                  "
                  :title="section.title"
                  @click="selectSection(group.chapter.id, section.id)"
                >
                  {{ section.title }}
                </button>
              </div>
            </div>
          </div>
        </Card>
      </aside>

      <!-- ==================== 右侧阅读区 ==================== -->
      <div class="flex min-w-0 flex-1 flex-col">
        <Card class="relative flex h-[60vh] flex-col overflow-hidden p-0 lg:h-[calc(100vh_-_13rem)]">
          <!-- 回到顶部：定位在阅读卡片右下角，滚动超过 400px 才出现 -->
          <Button
            v-if="showTop"
            variant="primary"
            size="icon"
            title="回到顶部"
            class="absolute bottom-16 right-4 z-20 !rounded-full shadow-lg"
            @click="backToTop"
          >
            <ArrowUp class="h-4 w-4" />
          </Button>
          <div
            ref="scroller"
            class="min-h-0 flex-1 overflow-y-auto overscroll-contain p-5 sm:p-6"
            @scroll.passive="onScroll"
          >
            <!-- 章节头 -->
            <div class="mb-5 flex flex-wrap items-center gap-2 border-b border-[var(--card-border)] pb-4">
              <BookOpen class="h-4 w-4 shrink-0 text-[var(--primary)]" />
              <h2 class="min-w-0 text-base font-bold text-[var(--text-strong)]">{{ currentChapter.title }}</h2>
              <Badge tone="primary">{{ currentChapter.sections.length }} 个小节</Badge>
            </div>

            <!-- 小节列表 -->
            <div class="space-y-8">
              <section v-for="section in renderSections" :id="section.id" :key="section.id" class="min-w-0">
                <h3 class="mb-3 text-lg font-semibold text-[var(--text-strong)]">{{ section.title }}</h3>

                <div class="space-y-3">
                  <template v-for="(b, i) in section.blocks" :key="i">
                    <!-- 段落 -->
                    <p v-if="b.kind === 'p'" class="min-w-0 text-sm leading-7 break-words text-[var(--text)]">{{ b.text }}</p>

                    <!-- 列表 -->
                    <ul v-else-if="b.kind === 'list'" class="space-y-2">
                      <li v-for="(item, j) in b.items" :key="j" class="flex gap-2.5 text-sm leading-6 text-[var(--text)]">
                        <span class="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--primary)]" />
                        <span class="min-w-0 flex-1 break-words">{{ item }}</span>
                      </li>
                    </ul>

                    <!-- 提示卡 -->
                    <div v-else-if="b.kind === 'note'" class="min-w-0 rounded-xl border-l-4 p-3.5" :class="b.boxClass">
                      <div class="flex flex-wrap items-center gap-1.5">
                        <span class="shrink-0 text-sm leading-none">{{ b.icon }}</span>
                        <span v-if="b.title" class="text-sm font-semibold text-[var(--text-strong)]">{{ b.title }}</span>
                      </div>
                      <p class="mt-1 text-sm leading-6 break-words text-[var(--text)]">{{ b.text }}</p>
                    </div>

                    <!-- 两列表格 -->
                    <div v-else class="overflow-hidden rounded-xl border border-[var(--card-border)]">
                      <div
                        v-for="(row, j) in b.rows"
                        :key="j"
                        class="flex flex-wrap gap-x-4 gap-y-1 px-3.5 py-2.5 text-sm"
                        :class="j > 0 ? 'border-t border-[var(--card-border)]' : ''"
                      >
                        <span class="w-28 shrink-0 text-[var(--muted)]">{{ row.k }}</span>
                        <span class="min-w-0 flex-1 break-all text-[var(--text)]">{{ row.v }}</span>
                      </div>
                    </div>
                  </template>
                </div>
              </section>

              <EmptyState v-if="renderSections.length === 0" icon="📖" text="本章暂无内容" />
            </div>

            <div class="h-6" />
          </div>

          <!-- 底部翻章 -->
          <div class="flex flex-wrap items-center gap-2 border-t border-[var(--card-border)] bg-[var(--card-soft)] px-4 py-3">
            <Button
              variant="gray"
              size="sm"
              :disabled="currentIndex <= 0"
              @click="gotoChapter(currentIndex - 1)"
            >
              <ChevronLeft class="h-3.5 w-3.5" /> 上一章
            </Button>
            <span class="min-w-0 flex-1 truncate text-center text-xs text-[var(--muted)]">
              {{ currentIndex + 1 }} / {{ HELP_CHAPTERS.length }} · {{ currentChapter.title }}
            </span>
            <Button
              variant="gray"
              size="sm"
              :disabled="currentIndex < 0 || currentIndex >= HELP_CHAPTERS.length - 1"
              @click="gotoChapter(currentIndex + 1)"
            >
              下一章 <ChevronRight class="h-3.5 w-3.5" />
            </Button>
          </div>
        </Card>
      </div>
    </div>
  </div>
</template>
