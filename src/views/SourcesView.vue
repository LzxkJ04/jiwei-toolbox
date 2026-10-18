<script setup lang="ts">
/**
 * 软件源管理页
 * 左侧：源列表（启用开关 / 类型徽章 / 连通状态 / 默认标记 / 操作）
 * 右侧：选中源的详情、高级选项、连通性测试与保存删除
 * 后端命令全部内联 invoke，避免依赖 tauri.ts 里尚未注册的导出。
 */
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity, CheckCircle2, CircleDashed, Download, FolderOpen, Globe, HardDrive,
  Loader2, Plus, RefreshCw, Save, Star, Trash2, Upload, XCircle,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Badge from "../components/ui/Badge.vue";
import Switch from "../components/ui/Switch.vue";
import Dialog from "../components/ui/Dialog.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { chooseDir, chooseFile } from "../lib/tauri";
import { toast } from "../lib/toast";
import { formatBytes } from "../lib/utils";

/** 与后端 SoftwareSource 字段一一对应（后端未做 camelCase 重命名，故用 snake_case） */
interface SoftwareSource {
  id: string;
  name: string;
  kind: "online" | "local" | string;
  url: string;
  description: string;
  enabled: boolean;
  builtin: boolean;
  use_global_proxy: boolean;
  verify_hash: boolean;
  filter_enabled: boolean;
}

interface TestResult {
  ok: boolean;
  elapsed_ms: number;
  package_count: number;
  message: string;
}

interface ScanResult {
  files: { name: string; size: number; path: string }[];
  total: number;
}

const KIND_OPTIONS = [
  { value: "online", label: "在线源" },
  { value: "local", label: "本地源" },
];

/* ==================== 列表与选中 ==================== */

const sources = ref<SoftwareSource[]>([]);
const loading = ref(false);
const selectedId = ref("");
const defaultId = ref("");
/** 详情区的可编辑副本，保存前不污染列表 */
const draft = ref<SoftwareSource | null>(null);
const saving = ref(false);
/** id -> 测试结果 */
const testResults = ref<Record<string, TestResult>>({});
const testingId = ref("");
/** 本地源目录扫描结果（详情区） */
const localScan = ref<ScanResult | null>(null);

const isBuiltin = computed(() => !!draft.value?.builtin);
/** 详情区当前源的测试结果 */
const detailStatus = computed<TestResult | null>(() =>
  draft.value ? testResults.value[draft.value.id] ?? null : null
);
/** 本地扫描的安装包总大小 */
const scanTotalSize = computed(() =>
  (localScan.value?.files ?? []).reduce((sum, f) => sum + (f.size || 0), 0)
);
const formScanTotalSize = computed(() =>
  (formScan.value?.files ?? []).reduce((sum, f) => sum + (f.size || 0), 0)
);

async function invokeList(): Promise<SoftwareSource[]> {
  return (await invoke<SoftwareSource[]>("list_sources")) ?? [];
}

async function load(keepId = true) {
  loading.value = true;
  try {
    const list = await invokeList();
    sources.value = list;
    try {
      const def = await invoke<SoftwareSource | null>("get_default_source");
      defaultId.value = def?.id ?? "";
    } catch {
      defaultId.value = "";
    }
    const stillThere = keepId && list.some((s) => s.id === selectedId.value);
    if (stillThere) {
      selectSource(selectedId.value);
    } else if (list.length > 0) {
      selectSource(list[0].id);
    } else {
      selectedId.value = "";
      draft.value = null;
      localScan.value = null;
    }
  } catch (err) {
    toast(`读取软件源失败：${String(err)}`, "error");
  } finally {
    loading.value = false;
  }
}

function selectSource(id: string) {
  selectedId.value = id;
  const hit = sources.value.find((s) => s.id === id);
  draft.value = hit ? { ...hit } : null;
  localScan.value = null;
  if (draft.value?.kind === "local") void scanDetailDir(draft.value.url);
}

/* ==================== 行内操作 ==================== */

async function toggleEnabled(row: SoftwareSource, value: boolean) {
  const next = { ...row, enabled: value };
  try {
    sources.value = await invoke<SoftwareSource[]>("save_source", { source: next });
    if (selectedId.value === row.id && draft.value) draft.value.enabled = value;
    toast(value ? `已启用「${row.name}」` : `已停用「${row.name}」`, "success");
  } catch (err) {
    toast(`操作失败：${String(err)}`, "error");
  }
}

async function makeDefault(row: SoftwareSource) {
  try {
    await invoke("set_default_source", { id: row.id });
    defaultId.value = row.id;
    toast(`已将「${row.name}」设为默认源`, "success");
  } catch (err) {
    toast(`设置默认源失败：${String(err)}`, "error");
  }
}

/* ==================== 详情：测试与保存 ==================== */

async function testSource(id: string) {
  if (!id || testingId.value) return;
  testingId.value = id;
  try {
    const res = await invoke<TestResult>("test_source", { id });
    testResults.value = { ...testResults.value, [id]: res };
    toast(res.ok ? res.message : `测试失败：${res.message}`, res.ok ? "success" : "error");
  } catch (err) {
    toast(`测试失败：${String(err)}`, "error");
  } finally {
    testingId.value = "";
  }
}

async function saveDetail() {
  const d = draft.value;
  if (!d) return;
  if (!d.name.trim()) {
    toast("请填写源名称", "warning");
    return;
  }
  if (!d.url.trim()) {
    toast(d.kind === "local" ? "请选择本地目录" : "请填写清单地址", "warning");
    return;
  }
  saving.value = true;
  try {
    sources.value = await invoke<SoftwareSource[]>("save_source", { source: d });
    toast("软件源已保存", "success");
    selectSource(selectedId.value);
  } catch (err) {
    toast(`保存失败：${String(err)}`, "error");
  } finally {
    saving.value = false;
  }
}

/* ==================== 本地目录 ==================== */

async function scanDetailDir(path: string) {
  localScan.value = null;
  if (!path.trim()) return;
  try {
    localScan.value = await invoke<ScanResult>("scan_local_source", { path });
  } catch {
    // 目录还没建好或不存在时静默，保存时后端会给出明确错误
  }
}

async function browseDetailDir() {
  const dir = await chooseDir("选择本地软件源目录");
  if (!dir || !draft.value) return;
  draft.value.url = dir;
  await scanDetailDir(dir);
}

/* ==================== 删除 ==================== */

const pendingDelete = ref<SoftwareSource | null>(null);

async function confirmDelete() {
  const target = pendingDelete.value;
  pendingDelete.value = null;
  if (!target) return;
  try {
    sources.value = await invoke<SoftwareSource[]>("delete_source", { id: target.id });
    toast(`已删除「${target.name}」`, "success");
    if (defaultId.value === target.id) defaultId.value = "";
    if (selectedId.value === target.id) {
      selectedId.value = "";
      draft.value = null;
      localScan.value = null;
      if (sources.value.length > 0) selectSource(sources.value[0].id);
    }
  } catch (err) {
    toast(String(err), "error");
  }
}

/* ==================== 导入导出 ==================== */

async function doExport() {
  try {
    const res = await invoke<{ file: string }>("export_sources");
    toast(`已导出到：${res.file}`, "success");
  } catch (err) {
    toast(`导出失败：${String(err)}`, "error");
  }
}

async function doImport() {
  const file = await chooseFile("选择软件源清单", "JSON 文件", "json");
  if (!file) return;
  try {
    sources.value = await invoke<SoftwareSource[]>("import_sources", { file });
    toast("软件源已导入", "success");
    const first = sources.value.find((s) => s.id === selectedId.value) ?? sources.value[0];
    selectSource(first ? first.id : "");
    try {
      const def = await invoke<SoftwareSource | null>("get_default_source");
      defaultId.value = def?.id ?? "";
    } catch {
      /* 忽略默认源读取失败 */
    }
  } catch (err) {
    toast(`导入失败：${String(err)}`, "error");
  }
}

/* ==================== 新增源弹窗 ==================== */

function blankSource(): SoftwareSource {
  return {
    id: "",
    name: "",
    kind: "online",
    url: "",
    description: "",
    enabled: true,
    builtin: false,
    use_global_proxy: true,
    verify_hash: true,
    filter_enabled: false,
  };
}

const createOpen = ref(false);
const creating = ref(false);
const form = ref<SoftwareSource>(blankSource());
const formTest = ref<TestResult | null>(null);
const formTesting = ref(false);
const formScan = ref<ScanResult | null>(null);

function openCreate() {
  form.value = blankSource();
  formTest.value = null;
  formScan.value = null;
  createOpen.value = true;
}

async function browseFormDir() {
  const dir = await chooseDir("选择本地软件源目录");
  if (!dir) return;
  form.value.url = dir;
  formScan.value = null;
  try {
    formScan.value = await invoke<ScanResult>("scan_local_source", { path: dir });
  } catch {
    /* 扫描失败交给保存时的校验提示 */
  }
}

async function testForm() {
  if (!form.value.url.trim()) {
    toast(form.value.kind === "local" ? "请先选择目录" : "请先填写清单地址", "warning");
    return;
  }
  formTesting.value = true;
  formTest.value = null;
  try {
    formTest.value = await invoke<TestResult>("test_source_connectivity", {
      url: form.value.url.trim(),
      kind: form.value.kind,
    });
  } catch (err) {
    toast(`测试失败：${String(err)}`, "error");
  } finally {
    formTesting.value = false;
  }
}

async function createSource() {
  const f = form.value;
  if (!f.name.trim()) {
    toast("请填写源名称", "warning");
    return;
  }
  if (!f.url.trim()) {
    toast(f.kind === "local" ? "请选择本地目录" : "请填写清单地址", "warning");
    return;
  }
  creating.value = true;
  try {
    const list = await invoke<SoftwareSource[]>("save_source", { source: f });
    sources.value = list;
    createOpen.value = false;
    toast(`「${f.name}」已添加`, "success");
    const added = list.find((s) => s.name === f.name.trim() && s.url === f.url.trim());
    if (added) selectSource(added.id);
  } catch (err) {
    toast(`保存失败：${String(err)}`, "error");
  } finally {
    creating.value = false;
  }
}

/* ==================== 展示辅助 ==================== */

function statusOf(id: string): TestResult | undefined {
  return testResults.value[id];
}

function kindLabel(kind: string): string {
  return kind === "local" ? "本地源" : "在线源";
}

onMounted(() => void load(false));
</script>

<template>
  <div>
    <PageHeader
      title="软件源管理"
      desc="管理软件清单来自哪里：在线清单可随时更新，本地目录适合内网或离线批量装机。"
    >
      <template #extra>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="gray" size="sm" :disabled="loading" @click="load()">
            <RefreshCw class="h-3.5 w-3.5" :class="loading && 'animate-spin'" /> 刷新
          </Button>
          <Button variant="gray" size="sm" @click="doExport">
            <Download class="h-3.5 w-3.5" /> 导出源清单
          </Button>
          <Button variant="gray" size="sm" @click="doImport">
            <Upload class="h-3.5 w-3.5" /> 导入源清单
          </Button>
          <Button variant="primary" size="sm" @click="openCreate">
            <Plus class="h-3.5 w-3.5" /> 新增源
          </Button>
        </div>
      </template>
    </PageHeader>

    <Spinner v-if="loading && !sources.length" label="读取软件源…" />

    <div v-else class="grid grid-cols-1 gap-5 lg:grid-cols-[320px_1fr]">
      <!-- ==================== 左：源列表 ==================== -->
      <div class="space-y-2">
        <EmptyState
          v-if="!sources.length"
          icon="🗂"
          text="还没有软件源"
          sub="点右上角「新增源」添加在线清单或本地目录"
        />

        <div
          v-for="s in sources"
          :key="s.id"
          class="w-full min-w-0 cursor-pointer rounded-xl border p-3 text-left transition-colors"
          :class="selectedId === s.id
            ? 'border-[var(--primary)] bg-[var(--primary-soft)]'
            : 'border-[var(--card-border)] bg-[var(--card-soft)] hover:border-[var(--primary)]'"
          @click="selectSource(s.id)"
        >
          <div class="flex flex-wrap items-start gap-2">
            <div class="shrink-0" @click.stop>
              <Switch
                :checked="s.enabled"
                class="scale-90"
                @update:checked="(v: boolean) => toggleEnabled(s, v)"
              />
            </div>

            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-1.5">
                <span class="min-w-0 max-w-full truncate text-sm font-semibold">{{ s.name }}</span>
                <Badge :tone="s.kind === 'local' ? 'warning' : 'primary'">
                  <component :is="s.kind === 'local' ? HardDrive : Globe" class="h-3 w-3" />
                  {{ kindLabel(s.kind) }}
                </Badge>
                <Badge v-if="defaultId === s.id" tone="primary">
                  <Star class="h-3 w-3" /> 默认
                </Badge>
                <Badge v-if="s.builtin" tone="gray">内置</Badge>
              </div>

              <p class="mt-1 break-all text-[10px] leading-relaxed text-[var(--muted)]">
                {{ s.url || "（未填写地址）" }}
              </p>

              <div class="mt-1.5 flex flex-wrap items-center gap-1.5 text-[10px]">
                <span
                  v-if="!statusOf(s.id)"
                  class="flex items-center gap-1 text-[var(--muted)]"
                >
                  <CircleDashed class="h-3 w-3" /> 未测试
                </span>
                <span v-else-if="statusOf(s.id)!.ok" class="flex items-center gap-1 text-[#30a46c]">
                  <CheckCircle2 class="h-3 w-3" /> 正常 · {{ statusOf(s.id)!.elapsed_ms }} ms ·
                  {{ statusOf(s.id)!.package_count }} 个包
                </span>
                <span v-else class="flex items-center gap-1 text-[#e5484d]">
                  <XCircle class="h-3 w-3" /> 不可用
                </span>
                <span v-if="!s.enabled" class="text-[var(--muted)]">已停用</span>
              </div>
            </div>
          </div>

          <div class="mt-2 flex flex-wrap items-center gap-1.5">
            <Button
              variant="ghost"
              size="sm"
              class="text-[var(--muted)]"
              :disabled="defaultId === s.id"
              title="设为默认源"
              @click.stop="makeDefault(s)"
            >
              <Star class="h-3 w-3" /> {{ defaultId === s.id ? "当前默认" : "设为默认" }}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              class="text-[var(--muted)] hover:text-[#e5484d]"
              :disabled="s.builtin"
              :title="s.builtin ? '内置官方源不可删除' : '删除'"
              @click.stop="pendingDelete = s"
            >
              <Trash2 class="h-3 w-3" /> 删除
            </Button>
          </div>
        </div>
      </div>

      <!-- ==================== 右：详情 ==================== -->
      <div class="min-w-0 space-y-4">
        <EmptyState
          v-if="!draft"
          icon="👈"
          text="选择左侧的软件源查看详情"
          sub="也可以新增一个在线清单源或本地目录源"
        />

        <template v-else>
          <!-- 1. 基础信息 -->
          <Card class="p-5">
            <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
              <h3 class="flex min-w-0 items-center gap-2 text-sm font-semibold">
                <Activity class="h-4 w-4 shrink-0 text-[var(--primary)]" /> 基础信息
              </h3>
              <div class="flex flex-wrap gap-1.5">
                <Badge v-if="draft.builtin" tone="gray">内置源</Badge>
                <Badge v-if="defaultId === draft.id" tone="primary">默认源</Badge>
              </div>
            </div>

            <div class="space-y-3">
              <div>
                <p class="mb-1 text-xs text-[var(--muted)]">源名称</p>
                <Input v-model="draft.name" placeholder="如：公司内网软件库" />
              </div>

              <div>
                <p class="mb-1 text-xs text-[var(--muted)]">源类型</p>
                <div class="flex flex-wrap gap-2">
                  <button
                    v-for="opt in KIND_OPTIONS"
                    :key="opt.value"
                    type="button"
                    class="rounded-lg border px-3 py-1.5 text-xs transition-colors"
                    :class="[
                      draft.kind === opt.value
                        ? 'border-[var(--primary)] bg-[var(--primary-soft)] text-[var(--primary)]'
                        : 'border-[var(--card-border)] bg-[var(--btn-gray)] text-[var(--text)]',
                      (isBuiltin || draft.kind === opt.value) ? 'cursor-default' : 'cursor-pointer',
                    ]"
                    :disabled="isBuiltin"
                    @click="!isBuiltin && (draft.kind = opt.value)"
                  >
                    {{ opt.label }}
                  </button>
                </div>
                <p v-if="isBuiltin" class="mt-1 text-[10px] text-[var(--muted)]">
                  内置官方源的类型与地址不可修改，只能改名称、描述与开关。
                </p>
              </div>

              <div v-if="draft.kind === 'local'">
                <p class="mb-1 text-xs text-[var(--muted)]">本地目录</p>
                <div class="flex flex-wrap items-center gap-2">
                  <Input
                    v-model="draft.url"
                    placeholder="D:\软件安装包"
                    class="min-w-0 flex-1"
                    :disabled="isBuiltin"
                  />
                  <Button variant="gray" size="sm" class="shrink-0" :disabled="isBuiltin" @click="browseDetailDir">
                    <FolderOpen class="h-3.5 w-3.5" /> 浏览
                  </Button>
                </div>
                <p v-if="localScan" class="mt-1 text-[11px] text-[var(--muted)]">
                  该目录下有 {{ localScan.total }} 个安装包（exe / msi / zip / 7z），合计
                  {{ formatBytes(scanTotalSize) }}
                </p>
              </div>

              <div v-else>
                <p class="mb-1 text-xs text-[var(--muted)]">清单地址（JSON 数组）</p>
                <Input
                  v-model="draft.url"
                  placeholder="https://example.com/softwares.json"
                  :disabled="isBuiltin"
                />
              </div>

              <div>
                <p class="mb-1 text-xs text-[var(--muted)]">描述（可选）</p>
                <Input v-model="draft.description" placeholder="这个源收录了哪些软件" />
              </div>
            </div>
          </Card>

          <!-- 2. 高级选项 -->
          <Card class="p-5">
            <h3 class="mb-3 text-sm font-semibold">高级选项</h3>
            <div class="space-y-3">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="min-w-0">
                  <p class="text-sm">使用全局代理</p>
                  <p class="text-[10px] text-[var(--muted)]">关闭后该源强制直连，适合内网地址</p>
                </div>
                <Switch
                  :checked="draft.use_global_proxy"
                  @update:checked="(v: boolean) => draft && (draft.use_global_proxy = v)"
                />
              </div>
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="min-w-0">
                  <p class="text-sm">校验安装包哈希</p>
                  <p class="text-[10px] text-[var(--muted)]">下载完成后比对清单里的 sha256 / md5</p>
                </div>
                <Switch
                  :checked="draft.verify_hash"
                  @update:checked="(v: boolean) => draft && (draft.verify_hash = v)"
                />
              </div>
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="min-w-0">
                  <p class="text-sm">启用源过滤</p>
                  <p class="text-[10px] text-[var(--muted)]">启用后该源的黑名单规则生效</p>
                </div>
                <Switch
                  :checked="draft.filter_enabled"
                  @update:checked="(v: boolean) => draft && (draft.filter_enabled = v)"
                />
              </div>
            </div>
          </Card>

          <!-- 3. 连通性测试 -->
          <Card class="p-5">
            <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
              <h3 class="text-sm font-semibold">连通性测试</h3>
              <Button
                variant="gray"
                size="sm"
                :disabled="testingId === draft.id || !draft.id"
                @click="testSource(draft.id)"
              >
                <Loader2 v-if="testingId === draft.id" class="h-3.5 w-3.5 animate-spin" />
                <Activity v-else class="h-3.5 w-3.5" />
                测试连通性
              </Button>
            </div>

            <p v-if="!draft.id" class="text-[11px] text-[var(--muted)]">新源保存后才能测试已保存的配置。</p>
            <p v-else-if="!detailStatus" class="text-[11px] text-[var(--muted)]">
              尚未测试。在线源会请求一次清单地址，本地源会扫描目录里的 JSON 清单。
            </p>
            <div v-else class="rounded-lg bg-[var(--card-soft)] p-3">
              <div class="flex flex-wrap items-center gap-2">
                <Badge :tone="detailStatus.ok ? 'success' : 'danger'">
                  <component :is="detailStatus.ok ? CheckCircle2 : XCircle" class="h-3 w-3" />
                  {{ detailStatus.ok ? "正常" : "不可用" }}
                </Badge>
                <span class="text-[11px] text-[var(--muted)]">
                  延迟 {{ detailStatus.elapsed_ms }} ms
                </span>
                <span class="text-[11px] text-[var(--muted)]">
                  包数量 {{ detailStatus.package_count }}
                </span>
              </div>
              <p class="mt-1.5 break-all text-[11px] leading-relaxed text-[var(--text)]">
                {{ detailStatus.message }}
              </p>
            </div>
          </Card>

          <!-- 4. 底部操作 -->
          <div class="flex flex-wrap items-center justify-end gap-2">
            <Button
              variant="danger"
              :disabled="draft.builtin"
              :title="draft.builtin ? '内置官方源不可删除' : '删除该源'"
              @click="pendingDelete = draft"
            >
              <Trash2 class="h-4 w-4" /> 删除
            </Button>
            <Button variant="primary" :disabled="saving" @click="saveDetail">
              <Loader2 v-if="saving" class="h-4 w-4 animate-spin" />
              <Save v-else class="h-4 w-4" />
              保存
            </Button>
          </div>
        </template>
      </div>
    </div>

    <!-- ==================== 新增源弹窗 ==================== -->
    <Dialog :open="createOpen" title="新增软件源" class="!max-w-xl" @update:open="createOpen = $event">
      <div class="space-y-3">
        <div>
          <p class="mb-1 text-xs text-[var(--muted)]">源名称</p>
          <Input v-model="form.name" placeholder="如：公司内网软件库" />
        </div>

        <div>
          <p class="mb-1 text-xs text-[var(--muted)]">源类型</p>
          <div class="flex flex-wrap gap-2">
            <button
              v-for="opt in KIND_OPTIONS"
              :key="opt.value"
              type="button"
              class="rounded-lg border px-3 py-1.5 text-xs transition-colors"
              :class="form.kind === opt.value
                ? 'border-[var(--primary)] bg-[var(--primary-soft)] text-[var(--primary)]'
                : 'border-[var(--card-border)] bg-[var(--btn-gray)] text-[var(--text)]'"
              @click="form.kind = opt.value; formTest = null"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>

        <div v-if="form.kind === 'local'">
          <p class="mb-1 text-xs text-[var(--muted)]">本地目录</p>
          <div class="flex flex-wrap items-center gap-2">
            <Input v-model="form.url" placeholder="D:\软件安装包" class="min-w-0 flex-1" />
            <Button variant="gray" size="sm" class="shrink-0" @click="browseFormDir">
              <FolderOpen class="h-3.5 w-3.5" /> 浏览
            </Button>
          </div>
          <p v-if="formScan" class="mt-1 text-[11px] text-[var(--muted)]">
            该目录下有 {{ formScan.total }} 个安装包（exe / msi / zip / 7z），合计
            {{ formatBytes(formScanTotalSize) }}
          </p>
        </div>
        <div v-else>
          <p class="mb-1 text-xs text-[var(--muted)]">清单地址（JSON 数组）</p>
          <Input v-model="form.url" placeholder="https://example.com/softwares.json" />
        </div>

        <div>
          <p class="mb-1 text-xs text-[var(--muted)]">描述（可选）</p>
          <Input v-model="form.description" placeholder="这个源收录了哪些软件" />
        </div>

        <div class="space-y-2.5 rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="text-xs">使用全局代理</span>
            <Switch
              :checked="form.use_global_proxy"
              @update:checked="(v: boolean) => (form.use_global_proxy = v)"
            />
          </div>
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="text-xs">校验安装包哈希</span>
            <Switch
              :checked="form.verify_hash"
              @update:checked="(v: boolean) => (form.verify_hash = v)"
            />
          </div>
        </div>

        <div>
          <Button variant="gray" size="sm" :disabled="formTesting" @click="testForm">
            <Loader2 v-if="formTesting" class="h-3.5 w-3.5 animate-spin" />
            <Activity v-else class="h-3.5 w-3.5" />
            测试连通性
          </Button>
          <div v-if="formTest" class="mt-2 rounded-lg bg-[var(--card-soft)] p-2.5">
            <div class="flex flex-wrap items-center gap-2">
              <Badge :tone="formTest.ok ? 'success' : 'danger'">
                <component :is="formTest.ok ? CheckCircle2 : XCircle" class="h-3 w-3" />
                {{ formTest.ok ? "正常" : "不可用" }}
              </Badge>
              <span class="text-[11px] text-[var(--muted)]">延迟 {{ formTest.elapsed_ms }} ms</span>
              <span class="text-[11px] text-[var(--muted)]">包数量 {{ formTest.package_count }}</span>
            </div>
            <p class="mt-1.5 break-all text-[11px] leading-relaxed">{{ formTest.message }}</p>
          </div>
        </div>
      </div>

      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="createOpen = false">取消</Button>
          <Button variant="primary" :disabled="creating" @click="createSource">
            <Loader2 v-if="creating" class="h-4 w-4 animate-spin" />
            <Save v-else class="h-4 w-4" />
            保存源
          </Button>
        </div>
      </template>
    </Dialog>

    <ConfirmDialog
      :open="!!pendingDelete"
      title="删除软件源？"
      :message="pendingDelete ? `将删除「${pendingDelete.name}」。删除后该源不再参与软件清单合并，已下载的安装包不受影响。` : ''"
      confirm-text="删除"
      danger
      @update:open="(v) => !v && (pendingDelete = null)"
      @confirm="confirmDelete"
    />
  </div>
</template>
