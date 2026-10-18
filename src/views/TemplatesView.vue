<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  Copy, FileDown, LayoutTemplate, Loader2, Pencil, Play, Plus, RefreshCw,
  Save, Search, Trash2,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Dialog from "../components/ui/Dialog.vue";
import Checkbox from "../components/ui/Checkbox.vue";
import Switch from "../components/ui/Switch.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import {
  deleteTemplate, duplicateTemplate, exportTemplateMarkdown, listTemplates, loadTemplate,
  saveTemplate,
  type SoftwareItem, type TaskTemplate, type TemplateMeta,
} from "../lib/tauri";
import { useSoftwareStore } from "../stores/software";
import { useRouteStore } from "../stores/route";
import { copyText } from "../lib/utils";
import { toast } from "../lib/toast";

const store = useSoftwareStore();
const route = useRouteStore();

const metas = ref<TemplateMeta[] | null>(null);
const loading = ref(false);
const filterTag = ref("all");
const selected = ref<string | null>(null);
const detail = ref<TaskTemplate | null>(null);
const detailLoading = ref(false);

async function refresh() {
  loading.value = true;
  metas.value = (await listTemplates()) ?? [];
  loading.value = false;
  if (metas.value.length > 0 && !metas.value.some((m) => m.name === selected.value)) {
    void openDetail(metas.value[0].name);
  } else if (metas.value.length === 0) {
    selected.value = null;
    detail.value = null;
  }
}

async function openDetail(name: string) {
  selected.value = name;
  detailLoading.value = true;
  try {
    detail.value = await loadTemplate(name);
  } catch (err) {
    toast(`读取模板失败：${String(err)}`, "error");
    detail.value = null;
  } finally {
    detailLoading.value = false;
  }
}

/** 所有标签汇总，用于筛选 */
const allTags = computed(() => {
  const set = new Set<string>();
  for (const m of metas.value ?? []) for (const t of m.tags) if (t.trim()) set.add(t);
  return [...set];
});

const tagOptions = computed(() => [
  { value: "all", label: "全部标签" },
  ...allTags.value.map((t) => ({ value: t, label: t })),
]);

const shownMetas = computed(() => {
  const list = metas.value ?? [];
  if (filterTag.value === "all") return list;
  return list.filter((m) => m.tags.includes(filterTag.value));
});

/* ============ 编辑器 ============ */

const editorOpen = ref(false);
const editing = ref<TaskTemplate | null>(null);
const renameFrom = ref<string | undefined>(undefined);
const editorSearch = ref("");
const saving = ref(false);

function blankTemplate(): TaskTemplate {
  return {
    name: "",
    description: "",
    tags: [],
    items: [],
    options: {
      auto_install: true,
      fail_policy: "stop",
      auto_shutdown: false,
      load_action: "add",
    },
    created: "",
    updated: "",
  };
}

function openNew() {
  editing.value = blankTemplate();
  renameFrom.value = undefined;
  editorSearch.value = "";
  editorOpen.value = true;
}

function openEdit(t: TaskTemplate) {
  // 深拷贝，取消时不污染列表
  editing.value = JSON.parse(JSON.stringify(t)) as TaskTemplate;
  renameFrom.value = t.name;
  editorSearch.value = "";
  editorOpen.value = true;
}

/** 编辑器里可选的软件（按搜索过滤，排除已加的） */
const pickable = computed(() => {
  const added = new Set((editing.value?.items ?? []).map((i) => i.id));
  const q = editorSearch.value.trim().toLowerCase();
  return (store.items ?? []).filter((i) => {
    if (added.has(i.id)) return false;
    if (!q) return true;
    return `${i.name} ${i.category}`.toLowerCase().includes(q);
  });
});

function addItem(item: SoftwareItem) {
  if (!editing.value) return;
  editing.value.items.push({
    id: item.id,
    name: item.name,
    silent_args: "",
    install_dir: "",
    disabled: false,
  });
}

function removeItem(idx: number) {
  editing.value?.items.splice(idx, 1);
}

/** 标签输入：逗号或空格分隔 */
const tagInput = ref("");
function applyTags() {
  if (!editing.value) return;
  const list = tagInput.value
    .split(/[,，\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  if (list.length) {
    editing.value.tags = [...new Set([...editing.value.tags, ...list])];
    tagInput.value = "";
  }
}

function removeTag(t: string) {
  if (!editing.value) return;
  editing.value.tags = editing.value.tags.filter((x) => x !== t);
}

async function doSave() {
  const t = editing.value;
  if (!t) return;
  if (!t.name.trim()) {
    toast("请填写模板名称", "warning");
    return;
  }
  if (t.items.length === 0) {
    toast("模板里至少要有一个软件", "warning");
    return;
  }
  saving.value = true;
  try {
    const name = await saveTemplate(t, renameFrom.value);
    toast(`模板「${name}」已保存`, "success");
    editorOpen.value = false;
    await refresh();
    await openDetail(name);
  } catch (err) {
    toast(`保存失败：${String(err)}`, "error");
  } finally {
    saving.value = false;
  }
}

/* ============ 删除 / 复制 / 导出 ============ */

const pendingDelete = ref<TemplateMeta | null>(null);

async function confirmDelete() {
  const m = pendingDelete.value;
  pendingDelete.value = null;
  if (!m) return;
  try {
    await deleteTemplate(m.name);
    toast(`已删除模板「${m.name}」`, "success");
    selected.value = null;
    detail.value = null;
    await refresh();
  } catch (err) {
    toast(`删除失败：${String(err)}`, "error");
  }
}

async function doDuplicate(name: string) {
  try {
    const n = await duplicateTemplate(name);
    toast(`已复制为「${n}」`, "success");
    await refresh();
  } catch (err) {
    toast(`复制失败：${String(err)}`, "error");
  }
}

async function doExportMarkdown(name: string) {
  try {
    const md = await exportTemplateMarkdown(name);
    const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${name}-装机清单.md`;
    a.click();
    URL.revokeObjectURL(a.href);
    toast("装机清单已导出", "success");
  } catch (err) {
    toast(`导出失败：${String(err)}`, "error");
  }
}

async function copyMarkdown(name: string) {
  try {
    const md = await exportTemplateMarkdown(name);
    const ok = await copyText(md);
    toast(ok ? "装机清单已复制到剪贴板" : "复制失败", ok ? "success" : "error");
  } catch (err) {
    toast(`生成失败：${String(err)}`, "error");
  }
}

/* ============ 加载到队列 ============ */

const loadingToQueue = ref(false);

/**
 * 把模板加载到任务队列。
 * 模板只存了软件 id，需要去软件库取完整条目；找不到的跳过并统计。
 */
async function loadToQueue(name: string) {
  const t = detail.value?.name === name ? detail.value : await loadTemplate(name);
  const enabled = t.items.filter((i) => !i.disabled);
  if (enabled.length === 0) {
    toast("模板内没有启用的软件", "warning");
    return;
  }
  loadingToQueue.value = true;
  try {
    const { useTasksStore } = await import("../stores/tasks");
    const tasks = useTasksStore();
    await store.load();
    const catalog = store.items ?? [];
    const picked: SoftwareItem[] = [];
    let missing = 0;
    for (const it of enabled) {
      const hit = catalog.find((c) => c.id === it.id);
      if (hit) picked.push(hit);
      else missing++;
    }
    const n = tasks.addFromSoftware(picked);
    if (n === 0 && picked.length > 0) {
      toast("模板内软件均已在任务队列中", "info");
    } else {
      toast(
        `已加入 ${n} 个软件到任务队列${missing > 0 ? `（${missing} 个在当前软件源不存在，已跳过）` : ""}`,
        missing > 0 ? "warning" : "success"
      );
    }
    if (t.options.auto_shutdown) await tasks.setFinishAction("shutdown");
    if (t.options.load_action === "start" && n > 0) {
      await tasks.start();
      toast("已按模板设置自动开始任务", "info");
    }
    route.go("tasks");
  } catch (err) {
    toast(`加载到队列失败：${String(err)}`, "error");
  } finally {
    loadingToQueue.value = false;
  }
}

function itemName(id: string) {
  return (store.items ?? []).find((s) => s.id === id)?.name;
}

onMounted(async () => {
  await store.load();
  await refresh();
});
</script>

<template>
  <div>
    <PageHeader title="批量任务模板" desc="把一套软件清单存成模板，装机或重装系统时一键加载到队列。">
      <template #extra>
        <Button variant="primary" size="sm" @click="openNew">
          <Plus class="h-3.5 w-3.5" /> 新建模板
        </Button>
        <Button variant="gray" size="sm" :disabled="loading" @click="refresh">
          <RefreshCw class="h-3.5 w-3.5" :class="loading && 'animate-spin'" /> 刷新
        </Button>
      </template>
    </PageHeader>

    <div class="grid grid-cols-1 gap-5 lg:grid-cols-[300px_1fr]">
      <!-- 左：模板列表 -->
      <div class="space-y-3">
        <Select v-if="allTags.length" v-model="filterTag" :options="tagOptions" class="w-full" />
        <Spinner v-if="loading && !metas" label="读取模板…" />
        <EmptyState
          v-else-if="!shownMetas.length"
          icon="📋"
          text="还没有任务模板"
          sub="点右上角「新建模板」，把常用软件组合保存下来"
        />
        <div v-else class="max-h-[70vh] space-y-2 overflow-y-auto pr-1">
          <button
            v-for="m in shownMetas"
            :key="m.name"
            type="button"
            class="w-full rounded-xl border p-3 text-left transition-colors"
            :class="selected === m.name
              ? 'border-[var(--primary)] bg-[var(--primary-soft)]'
              : 'border-[var(--card-border)] bg-[var(--card-soft)] hover:border-[var(--primary)]'"
            @click="openDetail(m.name)"
          >
            <p class="truncate text-sm font-semibold">{{ m.name }}</p>
            <p v-if="m.description" class="mt-0.5 truncate text-xs text-[var(--muted)]">{{ m.description }}</p>
            <div class="mt-1.5 flex flex-wrap items-center gap-1">
              <Badge tone="gray">{{ m.enabled_count }} 个软件</Badge>
              <Badge v-for="t in m.tags" :key="t" tone="primary">{{ t }}</Badge>
            </div>
            <p class="mt-1 text-[10px] text-[var(--muted)]">{{ m.updated }}</p>
          </button>
        </div>
      </div>

      <!-- 右：详情 -->
      <Card class="p-5">
        <Spinner v-if="detailLoading" label="读取模板详情…" />
        <EmptyState
          v-else-if="!detail"
          icon="👈"
          text="选择左侧的模板查看详情"
          sub="也可以新建一个模板"
        />
        <template v-else>
          <div class="mb-3 flex flex-wrap items-start justify-between gap-2">
            <div class="min-w-0">
              <h3 class="truncate text-lg font-bold">{{ detail.name }}</h3>
              <p v-if="detail.description" class="mt-1 text-xs text-[var(--muted)]">{{ detail.description }}</p>
            </div>
            <div class="flex flex-wrap gap-1.5">
              <Button variant="primary" size="sm" :disabled="loadingToQueue" @click="loadToQueue(detail.name)">
                <Loader2 v-if="loadingToQueue" class="h-3.5 w-3.5 animate-spin" />
                <Play v-else class="h-3.5 w-3.5" />
                加载到队列
              </Button>
              <Button variant="gray" size="sm" @click="openEdit(detail)">
                <Pencil class="h-3.5 w-3.5" /> 编辑
              </Button>
              <Button variant="gray" size="sm" @click="doDuplicate(detail.name)">
                <Copy class="h-3.5 w-3.5" /> 复制
              </Button>
              <Button variant="gray" size="sm" @click="doExportMarkdown(detail.name)">
                <FileDown class="h-3.5 w-3.5" /> 导出清单
              </Button>
              <Button variant="gray" size="sm" @click="copyMarkdown(detail.name)">
                <Copy class="h-3.5 w-3.5" /> 复制清单
              </Button>
              <Button variant="danger" size="sm" @click="pendingDelete = { name: detail.name, description: '', tags: [], count: 0, enabled_count: 0, created: '', updated: '' }">
                <Trash2 class="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>

          <div v-if="detail.tags.length" class="mb-3 flex flex-wrap gap-1">
            <Badge v-for="t in detail.tags" :key="t" tone="primary">{{ t }}</Badge>
          </div>

          <!-- 执行参数 -->
          <div class="mb-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">自动静默安装</p>
              <p class="text-sm font-semibold">{{ detail.options.auto_install ? "是" : "否" }}</p>
            </div>
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">失败策略</p>
              <p class="text-sm font-semibold">{{ detail.options.fail_policy === "skip" ? "跳过继续" : "停止队列" }}</p>
            </div>
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">完成后关机</p>
              <p class="text-sm font-semibold">{{ detail.options.auto_shutdown ? "是" : "否" }}</p>
            </div>
            <div class="rounded-lg bg-[var(--card-soft)] p-2 text-center">
              <p class="text-[10px] text-[var(--muted)]">加载后</p>
              <p class="text-sm font-semibold">{{ detail.options.load_action === "start" ? "自动开始" : "仅入队" }}</p>
            </div>
          </div>

          <!-- 软件清单 -->
          <p class="mb-2 text-xs font-semibold text-[var(--text-strong)]">
            软件清单（{{ detail.items.filter((i) => !i.disabled).length }} / {{ detail.items.length }} 启用）
          </p>
          <div class="divide-y divide-[var(--card-border)] rounded-xl border border-[var(--card-border)]">
            <div
              v-for="(it, i) in detail.items"
              :key="`${it.id}-${i}`"
              class="flex flex-wrap items-center gap-2 px-3 py-2 text-xs"
              :class="it.disabled && 'opacity-50'"
            >
              <span class="w-6 shrink-0 text-right tabular-nums text-[var(--muted)]">{{ i + 1 }}</span>
              <span class="min-w-0 flex-1 truncate font-medium">{{ itemName(it.id) || it.name || it.id }}</span>
              <span v-if="it.silent_args" class="shrink-0 rounded bg-[var(--btn-gray)] px-1.5 py-0.5 font-mono text-[10px]">{{ it.silent_args }}</span>
              <span v-if="!itemName(it.id)" class="shrink-0 text-[10px] text-[#f5a623]">当前源无此软件</span>
              <span v-if="it.disabled" class="shrink-0 text-[10px] text-[var(--muted)]">已禁用</span>
            </div>
          </div>
          <p class="mt-2 text-[10px] text-[var(--muted)]">
            模板只保存软件引用与参数，不绑定当前队列；修改模板不会影响已加载的任务。
          </p>
        </template>
      </Card>
    </div>

    <!-- 编辑器 -->
    <Dialog
      :open="editorOpen"
      :title="renameFrom ? '编辑模板' : '新建模板'"
      class="!max-w-3xl"
      @update:open="editorOpen = $event"
    >
      <div v-if="editing" class="space-y-3">
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">模板名称</p>
            <Input v-model="editing.name" placeholder="如：办公装机" />
          </div>
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">标签（回车添加）</p>
            <Input v-model="tagInput" placeholder="办公、开发…" @keyup.enter="applyTags" />
            <div v-if="editing.tags.length" class="mt-1 flex flex-wrap gap-1">
              <button
                v-for="t in editing.tags"
                :key="t"
                type="button"
                class="rounded-full bg-[var(--primary-soft)] px-2 py-0.5 text-[10px] text-[var(--primary)]"
                @click="removeTag(t)"
              >
                {{ t }} ×
              </button>
            </div>
          </div>
        </div>
        <div>
          <p class="mb-1 text-xs text-[var(--muted)]">描述（可选）</p>
          <Input v-model="editing.description" placeholder="这个模板适用于什么场景" />
        </div>

        <!-- 软件选择 -->
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div>
            <p class="mb-1 flex items-center gap-1 text-xs text-[var(--muted)]">
              <LayoutTemplate class="h-3 w-3" /> 从软件库添加
            </p>
            <div class="relative mb-1">
              <Search class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-[var(--muted)]" />
              <Input v-model="editorSearch" placeholder="搜索软件" class="!pl-8" />
            </div>
            <div class="max-h-56 space-y-1 overflow-y-auto rounded-lg border border-[var(--card-border)] p-1.5">
              <button
                v-for="s in pickable"
                :key="s.id"
                type="button"
                class="flex w-full items-center justify-between gap-2 rounded px-2 py-1 text-left text-xs hover:bg-[var(--card-soft)]"
                @click="addItem(s)"
              >
                <span class="truncate">{{ s.name }}</span>
                <Plus class="h-3 w-3 shrink-0 text-[var(--primary)]" />
              </button>
              <p v-if="!pickable.length" class="py-4 text-center text-xs text-[var(--muted)]">没有可添加的软件</p>
            </div>
          </div>
          <div>
            <p class="mb-1 text-xs text-[var(--muted)]">已选软件（{{ editing.items.length }}）</p>
            <div class="max-h-64 space-y-1.5 overflow-y-auto rounded-lg border border-[var(--card-border)] p-1.5">
              <div
                v-for="(it, i) in editing.items"
                :key="`${it.id}-${i}`"
                class="rounded bg-[var(--card-soft)] px-2 py-1.5"
                :class="it.disabled && 'opacity-50'"
              >
                <div class="flex items-center gap-2">
                  <Switch
                    :checked="!it.disabled"
                    class="scale-75"
                    @update:checked="(v: boolean) => (it.disabled = !v)"
                  />
                  <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ it.name || it.id }}</span>
                  <button type="button" class="shrink-0 text-[var(--muted)] hover:text-[#e5484d]" @click="removeItem(i)">
                    <Trash2 class="h-3 w-3" />
                  </button>
                </div>
                <div class="mt-1 flex gap-1">
                  <Input v-model="it.silent_args" placeholder="静默参数（可选）" class="!py-1 text-[11px]" />
                </div>
              </div>
              <p v-if="!editing.items.length" class="py-4 text-center text-xs text-[var(--muted)]">
                从左侧点软件加入
              </p>
            </div>
          </div>
        </div>

        <!-- 全局执行参数 -->
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-2.5">
          <p class="mb-1.5 text-xs font-semibold">全局执行参数</p>
          <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-xs">
            <label class="flex items-center gap-1.5">
              <Checkbox
                :checked="editing.options.auto_install"
                @update:checked="(v: boolean) => editing && (editing.options.auto_install = v)"
              />
              下载后自动静默安装
            </label>
            <label class="flex items-center gap-1.5">
              <Checkbox
                :checked="editing.options.auto_shutdown"
                @update:checked="(v: boolean) => editing && (editing.options.auto_shutdown = v)"
              />
              完成后自动关机
            </label>
            <div class="flex items-center gap-1.5">
              <span class="text-[var(--muted)]">失败策略</span>
              <Select
                :model-value="editing.options.fail_policy"
                :options="[{ value: 'stop', label: '停止队列' }, { value: 'skip', label: '跳过继续' }]"
                class="w-28"
                @update:model-value="(v: string) => editing && (editing.options.fail_policy = v)"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-[var(--muted)]">加载后</span>
              <Select
                :model-value="editing.options.load_action"
                :options="[{ value: 'add', label: '仅入队' }, { value: 'start', label: '自动开始' }]"
                class="w-28"
                @update:model-value="(v: string) => editing && (editing.options.load_action = v)"
              />
            </div>
          </div>
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-3">
          <Button variant="gray" @click="editorOpen = false">取消</Button>
          <Button variant="primary" :disabled="saving" @click="doSave">
            <Loader2 v-if="saving" class="h-4 w-4 animate-spin" />
            <Save v-else class="h-4 w-4" />
            保存模板
          </Button>
        </div>
      </template>
    </Dialog>

    <ConfirmDialog
      :open="!!pendingDelete"
      title="删除模板"
      :message="pendingDelete ? `删除模板「${pendingDelete.name}」后，保存的软件清单将永久丢失，不影响已执行或正在执行的任务。` : ''"
      confirm-text="确认删除"
      danger
      @update:open="(v) => !v && (pendingDelete = null)"
      @confirm="confirmDelete"
    />
  </div>
</template>
