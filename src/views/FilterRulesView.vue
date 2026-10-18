<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  Lock, Plus, RotateCcw, Save, ShieldAlert, ShieldCheck, Trash2,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Select from "../components/ui/Select.vue";
import Badge from "../components/ui/Badge.vue";
import Switch from "../components/ui/Switch.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import {
  getFilterRules, resetFilterRules, saveFilterRules,
  type FilterRule, type RuleSet,
} from "../lib/tauri";
import { toast } from "../lib/toast";

type Tab = "uninstall" | "download";
const tab = ref<Tab>("uninstall");

const rules = ref<RuleSet | null>(null);
const loading = ref(false);
const saving = ref(false);
const dirty = ref(false);

const uninstallUser = ref<FilterRule[]>([]);
const downloadRules = ref<FilterRule[]>([]);

const uninstallBuiltin = computed(() =>
  (rules.value?.uninstall ?? []).filter((r) => r.builtin)
);

const search = ref("");

const KIND_OPTS = [
  { value: "name", label: "软件名称" },
  { value: "path", label: "安装路径关键词" },
];

const draft = ref<{ keyword: string; kind: string; note: string }>({
  keyword: "",
  kind: "name",
  note: "",
});

const pendingDelete = ref<{ tab: Tab; index: number } | null>(null);
const showReset = ref(false);

function current(): FilterRule[] {
  return tab.value === "uninstall" ? uninstallUser.value : downloadRules.value;
}

const filtered = computed(() => {
  const kw = search.value.trim().toLowerCase();
  const list = current();
  if (!kw) return list;
  return list.filter((r) => `${r.keyword} ${r.note}`.toLowerCase().includes(kw));
});

const builtinShown = computed(() => {
  const kw = search.value.trim().toLowerCase();
  if (!kw) return uninstallBuiltin.value;
  return uninstallBuiltin.value.filter((r) =>
    `${r.keyword} ${r.note}`.toLowerCase().includes(kw)
  );
});

async function load() {
  loading.value = true;
  const res = await getFilterRules();
  loading.value = false;
  if (!res) return;
  rules.value = res;
  uninstallUser.value = res.uninstall.filter((r) => !r.builtin);
  downloadRules.value = res.download.map((r) => ({ ...r, builtin: false }));
  dirty.value = false;
}

function markDirty() {
  dirty.value = true;
}

function addRule() {
  const kw = draft.value.keyword.trim();
  if (!kw) {
    toast("关键词不能为空", "warning");
    return;
  }
  const list = current();
  if (list.some((r) => r.keyword.toLowerCase() === kw.toLowerCase() && r.kind === draft.value.kind)) {
    toast("已存在相同的规则", "info");
    return;
  }
  list.push({
    keyword: kw,
    kind: draft.value.kind,
    note: draft.value.note.trim(),
    enabled: true,
    builtin: false,
  });
  draft.value = { keyword: "", kind: draft.value.kind, note: "" };
  markDirty();
}

function toggleRule(r: FilterRule) {
  r.enabled = !r.enabled;
  markDirty();
}

async function confirmDelete() {
  const p = pendingDelete.value;
  pendingDelete.value = null;
  if (!p) return;
  const list = p.tab === "uninstall" ? uninstallUser : downloadRules;
  list.value.splice(p.index, 1);
  markDirty();
  toast("已移除，记得保存", "info");
}

async function persist() {
  saving.value = true;
  const ok = await saveFilterRules(uninstallUser.value, downloadRules.value);
  saving.value = false;
  if (ok === null) {
    toast("保存失败", "error");
    return;
  }
  dirty.value = false;
  toast("规则已保存，重新扫描本机软件后生效", "success");
}

async function doReset() {
  showReset.value = false;
  const ok = await resetFilterRules();
  if (ok === null) {
    toast("重置失败", "error");
    return;
  }
  uninstallUser.value = [];
  downloadRules.value = [];
  dirty.value = false;
  toast("已重置为默认（内置保护项保留）", "success");
}

onMounted(load);
</script>

<template>
  <div>
    <PageHeader
      title="黑白名单"
      desc="把不该动的软件藏起来，减少误删系统组件的风险。"
    >
      <template #extra>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="gray" size="sm" :disabled="loading" @click="showReset = true">
            <RotateCcw class="h-3.5 w-3.5" /> 重置
          </Button>
          <Button variant="primary" size="sm" :disabled="!dirty || saving" @click="persist">
            <Save class="h-3.5 w-3.5" /> {{ dirty ? "保存更改" : "已保存" }}
          </Button>
        </div>
      </template>
    </PageHeader>

    <Spinner v-if="loading && !rules" label="加载规则…" />

    <template v-else>
      <!-- 分栏 -->
      <div class="mb-4 flex flex-wrap gap-2">
        <button class="chip" :class="tab === 'uninstall' && 'chip-active'" @click="tab = 'uninstall'">
          卸载黑名单（{{ uninstallUser.length + uninstallBuiltin.length }}）
        </button>
        <button class="chip" :class="tab === 'download' && 'chip-active'" @click="tab = 'download'">
          下载过滤（{{ downloadRules.length }}）
        </button>
      </div>

      <div class="mb-4 flex flex-wrap items-center gap-3">
        <Input v-model="search" placeholder="搜索关键词或备注…" class="max-w-[280px]" />
        <span v-if="dirty" class="text-xs text-[#f59e0b]">有未保存的更改</span>
      </div>

      <!-- 卸载黑名单 -->
      <template v-if="tab === 'uninstall'">
        <!-- 内置保护 -->
        <Card class="mb-4 p-5">
          <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
            <h4 class="flex items-center gap-2 font-semibold">
              <Lock class="h-4 w-4 text-[#30a46c]" /> 内置系统保护
            </h4>
            <Badge tone="success">{{ uninstallBuiltin.length }} 条 · 不可删除</Badge>
          </div>
          <p class="mb-3 text-xs leading-relaxed text-[var(--muted)]">
            这些是 Windows 核心组件与常用运行库，删除会导致系统或大量软件无法运行。
            命中规则的软件不会出现在「本机软件」列表里，也就无法被选中卸载。
          </p>
          <div class="divide-y divide-[var(--card-border)] rounded-xl border border-[var(--card-border)]">
            <div
              v-for="r in builtinShown"
              :key="r.keyword"
              class="flex items-start gap-2.5 px-3 py-2"
            >
              <ShieldCheck class="mt-0.5 h-3.5 w-3.5 shrink-0 text-[#30a46c]" />
              <div class="min-w-0 flex-1">
                <p class="truncate text-sm" :title="r.keyword">{{ r.keyword }}</p>
                <p class="mt-0.5 truncate text-[10px] text-[var(--muted)]">
                  {{ r.kind === "path" ? "路径匹配" : "名称匹配" }} · {{ r.note }}
                </p>
              </div>
            </div>
          </div>
        </Card>

        <!-- 用户规则 -->
        <Card class="p-5">
          <h4 class="mb-3 font-semibold">自定义规则</h4>

          <div class="mb-3 flex flex-col gap-2 sm:flex-row">
            <Select v-model="draft.kind" :options="KIND_OPTS" class="sm:w-40" />
            <Input v-model="draft.keyword" placeholder="关键词，如 某厂商名" class="flex-1" />
            <Input v-model="draft.note" placeholder="备注（可选）" class="sm:w-48" />
            <Button variant="primary" size="sm" class="shrink-0" @click="addRule">
              <Plus class="h-3.5 w-3.5" /> 添加
            </Button>
          </div>

          <EmptyState
            v-if="filtered.length === 0"
            icon="🛡"
            text="还没有自定义规则"
            sub="填关键词后点添加，可保护特定软件不被误卸载"
          />
          <div v-else class="divide-y divide-[var(--card-border)] rounded-xl border border-[var(--card-border)]">
            <div
              v-for="(r, i) in filtered"
              :key="`${r.keyword}-${i}`"
              class="flex items-center gap-2.5 px-3 py-2"
            >
              <Switch :checked="r.enabled" @update:checked="toggleRule(r)" />
              <div class="min-w-0 flex-1">
                <p class="truncate text-sm" :class="!r.enabled && 'text-[var(--muted)] line-through'">
                  {{ r.keyword }}
                </p>
                <p class="mt-0.5 truncate text-[10px] text-[var(--muted)]">
                  {{ r.kind === "path" ? "路径匹配" : "名称匹配" }}{{ r.note ? ` · ${r.note}` : "" }}
                </p>
              </div>
              <Button
                variant="ghost"
                size="sm"
                class="text-[var(--muted)] hover:text-[#e5484d]"
                title="删除"
                @click="pendingDelete = { tab: 'uninstall', index: uninstallUser.indexOf(r) }"
              >
                <Trash2 class="h-3 w-3" />
              </Button>
            </div>
          </div>
        </Card>
      </template>

      <!-- 下载过滤 -->
      <template v-else>
        <Card class="p-5">
          <div class="mb-2 flex items-center gap-2">
            <ShieldAlert class="h-4 w-4 text-[#f59e0b]" />
            <h4 class="font-semibold">软件库过滤</h4>
          </div>
          <p class="mb-3 text-xs leading-relaxed text-[var(--muted)]">
            命中规则的软件不会出现在软件库，也无法加入下载队列。可用于隐藏不想看到的付费软件或某类工具。
          </p>

          <div class="mb-3 flex flex-col gap-2 sm:flex-row">
            <Select v-model="draft.kind" :options="KIND_OPTS" class="sm:w-40" />
            <Input v-model="draft.keyword" placeholder="软件名或分类关键词" class="flex-1" />
            <Input v-model="draft.note" placeholder="备注（可选）" class="sm:w-48" />
            <Button variant="primary" size="sm" class="shrink-0" @click="addRule">
              <Plus class="h-3.5 w-3.5" /> 添加
            </Button>
          </div>

          <EmptyState
            v-if="downloadRules.length === 0"
            icon="🔍"
            text="没有下载过滤规则"
            sub="软件库会显示全部软件"
          />
          <div v-else class="divide-y divide-[var(--card-border)] rounded-xl border border-[var(--card-border)]">
            <div
              v-for="(r, i) in downloadRules"
              :key="`${r.keyword}-${i}`"
              class="flex items-center gap-2.5 px-3 py-2"
            >
              <Switch :checked="r.enabled" @update:checked="toggleRule(r)" />
              <div class="min-w-0 flex-1">
                <p class="truncate text-sm" :class="!r.enabled && 'text-[var(--muted)] line-through'">
                  {{ r.keyword }}
                </p>
                <p class="mt-0.5 truncate text-[10px] text-[var(--muted)]">
                  {{ r.kind === "path" ? "分类匹配" : "名称匹配" }}{{ r.note ? ` · ${r.note}` : "" }}
                </p>
              </div>
              <Button
                variant="ghost"
                size="sm"
                class="text-[var(--muted)] hover:text-[#e5484d]"
                title="删除"
                @click="pendingDelete = { tab: 'download', index: i }"
              >
                <Trash2 class="h-3 w-3" />
              </Button>
            </div>
          </div>
        </Card>
      </template>

      <p class="mt-3 rounded-lg bg-[#f5a6231a] p-2.5 text-[11px] leading-relaxed text-[#b57d10]">
        匹配为「包含」判断且不区分大小写：填「Visual」会命中所有名字里含 Visual 的软件。
        关键词留空不会误伤全部软件，这类规则会被拒绝保存。
      </p>
    </template>

    <ConfirmDialog
      :open="!!pendingDelete"
      title="移除这条规则？"
      :message="pendingDelete
        ? `将移除「${(pendingDelete.tab === 'uninstall' ? uninstallUser : downloadRules)[pendingDelete.index]?.keyword ?? ''}」。移除后对应软件会重新出现在列表中。`
        : ''"
      confirm-text="移除"
      danger
      @update:open="(v) => !v && (pendingDelete = null)"
      @confirm="confirmDelete"
    />

    <ConfirmDialog
      :open="showReset"
      title="重置所有自定义规则？"
      message="将清空全部自定义的卸载黑名单与下载过滤规则。内置系统保护项不受影响，仍然生效。"
      confirm-text="重置"
      danger
      @update:open="(v) => !v && (showReset = false)"
      @confirm="doReset"
    />
  </div>
</template>
