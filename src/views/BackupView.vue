<script setup lang="ts">
// 清单备份：后端持久化（data\备份导出\*.jwinv.json）；导出/导入 JSON 文件用于跨电脑迁移。
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Archive, Download, FileUp, GitCompare, ListRestart, Pencil, RefreshCw, RotateCcw, Save, ShieldCheck, Trash2 } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Input from "../components/ui/Input.vue";
import Switch from "../components/ui/Switch.vue";
import Spinner from "../components/ui/Spinner.vue";
import Checkbox from "../components/ui/Checkbox.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import Dialog from "../components/ui/Dialog.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import Select from "../components/ui/Select.vue";
import {
  getAutoBackupInventory,
  listInstalled,
  setAutoBackupInventory,
  saveInventory,
  listInventories,
  loadInventoryDoc,
  deleteInventory,
  renameInventory,
  createBackup,
  listBackups,
  type BackupItem,
  type InventoryAppRef,
  type InventoryMeta,
} from "../lib/tauri";
import { useSoftwareStore } from "../stores/software";
import { formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

const store = useSoftwareStore();

/* ---- 左栏：本机软件清单 ---- */
const apps = ref<InventoryAppRef[] | null>(null);
const scanning = ref(false);
const appSearch = ref("");
const appChecked = ref(new Set<string>());

async function scanInstalled() {
  scanning.value = true;
  const list = await listInstalled();
  apps.value = (list ?? []).map((a) => ({
    name: a.name, version: a.version, publisher: a.publisher, install_location: a.install_location || "",
  }));
  scanning.value = false;
}

const filteredApps = computed(() => {
  const list = apps.value ?? [];
  const q = appSearch.value.trim().toLowerCase();
  if (!q) return list;
  return list.filter((a) => `${a.name} ${a.publisher}`.toLowerCase().includes(q));
});

function setAppChecked(name: string, v: boolean) {
  const next = new Set(appChecked.value);
  if (v) next.add(name);
  else next.delete(name);
  appChecked.value = next;
}

/* ---- 生成备份清单（弹窗自定义名称） ---- */
const showCreate = ref(false);
const createName = ref("");
const saving = ref(false);

function openCreate() {
  if (appChecked.value.size === 0) {
    toast("请先在左侧勾选要备份的软件", "info");
    return;
  }
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  createName.value = `软件清单-${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}`;
  showCreate.value = true;
}

async function confirmCreate() {
  const name = createName.value.trim();
  if (!name) {
    toast("请填写备份名称", "warning");
    return;
  }
  saving.value = true;
  const selected = (apps.value ?? []).filter((a) => appChecked.value.has(a.name));
  const res = await saveInventory(name, selected);
  saving.value = false;
  if (res === null) {
    toast("保存失败（名称非法或磁盘错误）", "error");
    return;
  }
  showCreate.value = false;
  appChecked.value = new Set();
  refreshInventories();
  toast(`备份清单「${name}」已保存（${selected.length} 个软件）`, "success");
}

/* ---- 右栏：历史备份文件 ---- */
const inventories = ref<InventoryMeta[]>([]);

function refreshInventories() {
  listInventories().then((v) => {
    inventories.value = v ?? [];
  });
}

/** 加载清单：按名称匹配软件库并勾选（跨页联动，软件库页可见） */
async function loadInventory(b: InventoryMeta) {
  const doc = await loadInventoryDoc(b.name);
  if (doc === null) {
    toast("清单文件损坏或不存在", "error");
    return;
  }
  await store.load();
  const items = store.items ?? [];
  const lower = (s: string) => (s || "").trim().toLowerCase();
  const matched: string[] = [];
  let miss = 0;
  for (const a of doc.apps) {
    const n = lower(a.name);
    if (!n) continue;
    const hit = items.find((i) => {
      const m = lower(i.name);
      return m && (m.includes(n) || n.includes(m));
    });
    if (hit) matched.push(hit.id);
    else miss++;
  }
  if (matched.length === 0) {
    toast("清单中的软件未在软件库中找到匹配项", "warning");
    return;
  }
  store.addChecked(matched);
  toast(
    `已加载「${b.name}」并勾选 ${matched.length} 个软件${miss > 0 ? `（${miss} 个未匹配，已跳过）` : ""}，可到软件库查看`,
    "success"
  );
}

/** 导出清单：JSON 文件下载（跨电脑迁移用） */
async function exportInventory(b: InventoryMeta) {
  const doc = await loadInventoryDoc(b.name);
  if (doc === null) {
    toast("清单文件损坏或不存在", "error");
    return;
  }
  const data = JSON.stringify(
    { type: "jw-software-inventory", name: doc.name, time: doc.created, apps: doc.apps },
    null,
    2
  );
  const blob = new Blob(["\ufeff" + data], { type: "application/json;charset=utf-8" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `${b.name}.json`;
  a.click();
  URL.revokeObjectURL(a.href);
  toast("清单已导出", "success");
}

/* ---- 清单对比：这份清单和本机现状差多少 ---- */
const compareOpen = ref(false);
const compareName = ref("");
const compareLoading = ref(false);
const compareResult = ref<{
  inList: { name: string; version: string }[];
  missing: { name: string; version: string }[];
  extra: { name: string; version: string }[];
  inLibrary: number;
} | null>(null);

const lower = (s: string) => (s || "").trim().toLowerCase();

/** 判断两份软件名是否指同一个软件（互包含即可，兼容中英文名差异） */
function sameApp(a: string, b: string) {
  const x = lower(a);
  const y = lower(b);
  if (!x || !y) return false;
  return x === y || x.includes(y) || y.includes(x);
}

async function compareInventory(b: InventoryMeta) {
  compareName.value = b.name;
  compareOpen.value = true;
  compareLoading.value = true;
  compareResult.value = null;
  const doc = await loadInventoryDoc(b.name);
  if (doc === null) {
    compareLoading.value = false;
    toast("清单文件损坏或不存在", "error");
    compareOpen.value = false;
    return;
  }
  // 本机现状：重新枚举，避免用旧缓存判断
  const installed = (await listInstalled()) ?? [];
  const installedNames = installed.map((a) => a.name);

  const missing = doc.apps.filter((a) => !installedNames.some((n) => sameApp(n, a.name)));
  // 清单之后新装、不在清单里的软件（只列名字，便于判断是否需要补进清单）
  const extra = installed
    .filter((a) => !doc.apps.some((d) => sameApp(a.name, d.name)))
    .map((a) => ({ name: a.name, version: a.version }));

  await store.load();
  const items = store.items ?? [];
  const inLibrary = doc.apps.filter((a) => items.some((i) => sameApp(i.name, a.name))).length;

  compareResult.value = {
    inList: doc.apps.map((a) => ({ name: a.name, version: a.version })),
    missing,
    extra: extra.slice(0, 200),
    inLibrary,
  };
  compareLoading.value = false;
}

/** 把清单里缺装的软件直接勾进软件库 */
async function selectMissing() {
  const r = compareResult.value;
  if (!r || r.missing.length === 0) return;
  await store.load();
  const items = store.items ?? [];
  const ids: string[] = [];
  let noMatch = 0;
  for (const m of r.missing) {
    const hit = items.find((i) => sameApp(i.name, m.name));
    if (hit) ids.push(hit.id);
    else noMatch++;
  }
  if (ids.length === 0) {
    toast("缺装的软件在软件库里都没有对应条目", "warning");
    return;
  }
  store.addChecked(ids);
  compareOpen.value = false;
  toast(
    `已勾选 ${ids.length} 个缺装软件，可到软件库加入队列${noMatch > 0 ? `（${noMatch} 个软件库没有）` : ""}`,
    "success"
  );
}

/* ---- 重命名 ---- */
const showRename = ref(false);
const renameTarget = ref<InventoryMeta | null>(null);
const renameName = ref("");

function openRename(b: InventoryMeta) {
  renameTarget.value = b;
  renameName.value = b.name;
  showRename.value = true;
}

async function confirmRename() {
  const name = renameName.value.trim();
  if (!name || !renameTarget.value) {
    toast("请填写名称", "warning");
    return;
  }
  const ok = await renameInventory(renameTarget.value.name, name);
  if (ok === null) {
    toast("重命名失败（目标名称已存在或名称非法）", "error");
    return;
  }
  refreshInventories();
  toast("已重命名", "success");
  showRename.value = false;
}

/* ---- 删除（确认） ---- */
const showDelete = ref(false);
const deleteTarget = ref<InventoryMeta | null>(null);

function openDelete(b: InventoryMeta) {
  deleteTarget.value = b;
  showDelete.value = true;
}

async function confirmDelete() {
  if (!deleteTarget.value) return;
  await deleteInventory(deleteTarget.value.name);
  refreshInventories();
  toast("备份清单已删除", "info");
  showDelete.value = false;
}

/* ---- 全局设置：自动备份清单 ---- */
const autoBackup = ref(false);
const autoBackupLoading = ref(true);

/* ---- 配置打包（.jwbak）：清单功能保持原样，这一段是下面新增的卡片 ---- */
interface ConfigProfile {
  id: string;
  name: string;
  paths: string[];
  exists: boolean;
}

interface VerifyResult {
  ok: boolean;
  checked: number;
  mismatches: string[];
}

const strategyOptions = [
  { value: "overwrite", label: "覆盖已有文件" },
  { value: "skip", label: "跳过已有文件" },
  { value: "rename", label: "重命名以免覆盖" },
];

const packNote = ref("");
const packing = ref(false);
const bakFiles = ref<BackupItem[]>([]);
const bakLoading = ref(false);
const strategy = ref("overwrite");
const verifyingFile = ref("");
const verifyText = ref("");
const verifyOk = ref<boolean | null>(null);
const showRestore = ref(false);
const restoreTarget = ref<BackupItem | null>(null);
const restoring = ref(false);

const profiles = ref<ConfigProfile[]>([]);
const profilesLoading = ref(false);
const profileChecked = ref(new Set<string>());
const profileNote = ref("");
const profileBacking = ref(false);

const strategyLabel = computed(() => strategyOptions.find((o) => o.value === strategy.value)?.label ?? "覆盖已有文件");

function errText(e: unknown) {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

function baseName(p: string) {
  const parts = p.split(/[/\\]/);
  return parts[parts.length - 1] || p;
}

async function refreshBakFiles() {
  bakLoading.value = true;
  const v = await listBackups();
  bakFiles.value = v ?? [];
  bakLoading.value = false;
}

async function packCurrentConfig() {
  packing.value = true;
  const res = await createBackup(packNote.value.trim());
  packing.value = false;
  if (res) {
    toast("当前配置已打包", "success");
    refreshBakFiles();
  }
}

async function loadProfiles() {
  profilesLoading.value = true;
  try {
    profiles.value = (await invoke<ConfigProfile[]>("list_config_profiles")) ?? [];
  } catch (e) {
    profiles.value = [];
    toast(`读取配置档案失败：${errText(e)}`, "error");
  } finally {
    profilesLoading.value = false;
  }
}

function setProfileChecked(id: string, v: boolean) {
  const next = new Set(profileChecked.value);
  if (v) next.add(id);
  else next.delete(id);
  profileChecked.value = next;
}

async function backupSelectedProfiles() {
  const ids = [...profileChecked.value];
  if (ids.length === 0) {
    toast("请先勾选要备份的软件", "info");
    return;
  }
  profileBacking.value = true;
  try {
    const res = await invoke<{ file: string }>("backup_profiles", { ids, note: profileNote.value.trim() });
    toast(`已备份所选软件配置：${baseName(res?.file ?? "")}`, "success");
    profileChecked.value = new Set();
    refreshBakFiles();
  } catch (e) {
    toast(`备份失败：${errText(e)}`, "error");
  } finally {
    profileBacking.value = false;
  }
}

async function verifyBak(b: BackupItem) {
  verifyingFile.value = b.file;
  verifyText.value = "";
  verifyOk.value = null;
  try {
    const res = await invoke<VerifyResult>("verify_backup", { file: b.file });
    verifyOk.value = !!res?.ok;
    if (res?.ok) {
      verifyText.value = `「${baseName(b.file)}」校验通过，已核对 ${res.checked} 个文件`;
      toast(verifyText.value, "success");
    } else {
      const detail = (res?.mismatches ?? []).slice(0, 3).join("；") || "校验未通过";
      verifyText.value = `「${baseName(b.file)}」校验未通过：${detail}`;
      toast(verifyText.value, "error");
    }
  } catch (e) {
    verifyOk.value = false;
    verifyText.value = `校验失败：${errText(e)}`;
    toast(verifyText.value, "error");
  } finally {
    verifyingFile.value = "";
  }
}

function openRestoreBak(b: BackupItem) {
  restoreTarget.value = b;
  showRestore.value = true;
}

async function confirmRestoreBak() {
  const b = restoreTarget.value;
  if (!b) return;
  restoring.value = true;
  try {
    await invoke("restore_backup", { file: b.file, strategy: strategy.value });
    toast(`已按「${strategyLabel.value}」恢复配置`, "success");
  } catch (e) {
    toast(`恢复失败：${errText(e)}`, "error");
  } finally {
    restoring.value = false;
  }
}

onMounted(async () => {
  void scanInstalled();
  refreshInventories();
  void refreshBakFiles();
  void loadProfiles();
  const v = await getAutoBackupInventory();
  if (v !== null) autoBackup.value = v;
  autoBackupLoading.value = false;
});

async function onAutoBackup(v: boolean) {
  autoBackup.value = v;
  const ok = await setAutoBackupInventory(v);
  toast(ok ? (v ? "已开启自动备份" : "已关闭自动备份") : "设置失败，请重试", ok ? "success" : "error");
  if (!ok) autoBackup.value = !v;
}
</script>

<template>
  <div>
    <PageHeader title="软件备份" desc="备份本机软件清单，跨电脑快速恢复安装选择" />

    <!-- 左右两板块 -->
    <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <!-- 左：本机软件清单备份 -->
      <Card class="flex flex-col p-5">
        <div class="mb-3 flex items-center justify-between gap-2">
          <div>
            <h3 class="font-semibold">本机软件清单备份</h3>
            <p class="mt-0.5 text-xs text-[var(--muted)]">
              从本机已注册软件中勾选，生成可迁移的备份清单（浏览器存储）
            </p>
          </div>
          <Button variant="gray" size="sm" :disabled="scanning" @click="scanInstalled">
            <RefreshCw class="h-3.5 w-3.5" /> {{ scanning ? "扫描中…" : "重新扫描" }}
          </Button>
        </div>

        <Input v-model="appSearch" placeholder="搜索本机软件名称或厂商…" class="mb-3" />

        <div v-if="apps === null" class="flex-1">
          <Spinner label="正在扫描本机软件…" />
        </div>
        <div v-else-if="filteredApps.length === 0" class="flex-1">
          <EmptyState icon="🔍" text="没有匹配的本机软件" sub="试试调整搜索词，或点击「重新扫描」" />
        </div>
        <div v-else class="max-h-[26rem] flex-1 space-y-1 overflow-y-auto pr-1">
          <label
            v-for="a in filteredApps"
            :key="a.name"
            class="flex cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors hover:bg-[var(--btn-gray)]"
          >
            <Checkbox :checked="appChecked.has(a.name)" @update:checked="(v) => setAppChecked(a.name, !!v)" />
            <span class="min-w-0 flex-1">
              <span class="block truncate text-sm">{{ a.name }}</span>
              <span class="block truncate text-xs text-[var(--muted)]">
                {{ a.version || "未知版本" }}<template v-if="a.publisher"> · {{ a.publisher }}</template>
              </span>
            </span>
          </label>
        </div>

        <div class="mt-3 flex items-center justify-between border-t border-[var(--card-border)] pt-3">
          <span class="text-xs text-[var(--muted)]">
            已勾选 <b class="text-[var(--primary)]">{{ appChecked.size }}</b> / {{ apps?.length ?? 0 }} 个软件
          </span>
          <Button variant="primary" size="sm" :disabled="appChecked.size === 0" @click="openCreate">
            <Save class="h-3.5 w-3.5" /> 生成备份清单
          </Button>
        </div>
      </Card>

      <!-- 右：历史备份文件 -->
      <Card class="flex flex-col p-5">
        <div class="mb-3">
          <h3 class="font-semibold">历史备份文件</h3>
          <p class="mt-0.5 text-xs text-[var(--muted)]">
            加载清单可在软件库中一键勾选对应软件；导出 JSON 可跨电脑导入
          </p>
        </div>

        <div v-if="inventories.length === 0" class="flex-1">
          <EmptyState icon="💾" text="暂无历史备份" sub="在左侧勾选软件并生成备份清单后，会显示在这里" />
        </div>
        <div v-else class="max-h-[26rem] flex-1 space-y-2.5 overflow-y-auto pr-1">
          <div
            v-for="b in inventories"
            :key="b.name"
            class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3.5"
          >
            <div class="flex flex-wrap items-center gap-2">
              <span class="truncate font-medium">{{ b.name }}</span>
              <Badge tone="gray">{{ b.count }} 个软件</Badge>
              <Badge tone="gray">{{ formatBytes(b.size) }}</Badge>
            </div>
            <p class="mt-1 text-xs text-[var(--muted)]">{{ b.created }}</p>
            <div class="mt-2.5 flex flex-wrap items-center gap-1.5">
              <Button variant="primary" size="sm" @click="loadInventory(b)">
                <FileUp class="h-3.5 w-3.5" /> 加载清单
              </Button>
              <Button variant="gray" size="sm" title="与本机已安装软件对比" @click="compareInventory(b)">
                <GitCompare class="h-3.5 w-3.5" /> 对比
              </Button>
              <Button variant="gray" size="sm" @click="exportInventory(b)">
                <Download class="h-3.5 w-3.5" /> 导出
              </Button>
              <Button variant="gray" size="sm" @click="openRename(b)">
                <Pencil class="h-3.5 w-3.5" /> 重命名
              </Button>
              <Button
                variant="ghost"
                size="icon"
                title="删除备份"
                class="ml-auto text-[var(--muted)] hover:text-[#e5484d]"
                @click="openDelete(b)"
              >
                <Trash2 class="h-4 w-4" />
              </Button>
            </div>
          </div>
        </div>
      </Card>
    </div>

    <!-- 全局设置卡 -->
    <Card class="mt-4 flex items-center justify-between gap-4 p-5">
      <div class="flex items-center gap-3">
        <ListRestart class="h-5 w-5 shrink-0 text-[var(--primary)]" />
        <div>
          <h3 class="text-sm font-semibold">自动备份软件清单</h3>
          <p class="mt-0.5 text-xs leading-relaxed text-[var(--muted)]">
            开启后程序启动时会自动在备份目录生成一份软件清单简易备份，无需手动操作。
          </p>
        </div>
      </div>
      <Switch v-if="!autoBackupLoading" :checked="autoBackup" @update:checked="onAutoBackup" />
      <span v-else class="text-xs text-[var(--muted)]">读取中…</span>
    </Card>

    <!-- 配置打包：位于软件清单备份下方，不改动上面的清单功能 -->
    <Card class="mt-4 p-5">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0">
          <h3 class="font-semibold">配置打包</h3>
          <p class="mt-0.5 text-xs leading-relaxed text-[var(--muted)]">
            把工具箱配置目录或常见软件的配置打成 .jwbak。恢复前会先核对清单里的 sha256，不一致则不会改动现有文件。软件配置会还原到工具箱配置目录下对应的子目录，不会写回 Chrome、微信等软件自己的原目录。
          </p>
        </div>
      </div>

      <div class="mt-3 flex flex-wrap items-center gap-2">
        <Input v-model="packNote" placeholder="当前配置的备注，可留空" class="min-w-[12rem] flex-1" />
        <Button variant="primary" size="sm" :disabled="packing" @click="packCurrentConfig">
          <Archive class="h-3.5 w-3.5" /> {{ packing ? "打包中…" : "打包当前配置" }}
        </Button>
        <Button variant="gray" size="sm" :disabled="bakLoading" @click="refreshBakFiles">
          <RefreshCw class="h-3.5 w-3.5" /> {{ bakLoading ? "刷新中…" : "刷新备份列表" }}
        </Button>
      </div>

      <div class="mt-3 flex flex-wrap items-center gap-2">
        <span class="text-xs text-[var(--muted)]">恢复策略</span>
        <Select v-model="strategy" :options="strategyOptions" class="min-w-[10rem]" />
        <span class="text-xs text-[var(--muted)]">当前：{{ strategyLabel }}</span>
      </div>

      <p
        v-if="verifyText"
        class="mt-2 truncate text-xs"
        :class="verifyOk === false ? 'font-medium text-[var(--text)]' : 'text-[var(--muted)]'"
        :title="verifyText"
      >
        {{ verifyText }}
      </p>

      <div v-if="bakFiles.length === 0" class="mt-3 text-sm text-[var(--muted)]">暂无 .jwbak 备份。打包后会出现在这里。</div>
      <div v-else class="mt-3 max-h-72 space-y-2 overflow-y-auto pr-1">
        <div
          v-for="b in bakFiles"
          :key="b.file"
          class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3"
        >
          <div class="flex flex-wrap items-center gap-2">
            <span class="min-w-0 flex-1 truncate text-sm font-medium" :title="b.file">{{ baseName(b.file) }}</span>
            <Badge tone="gray">{{ formatBytes(b.size) }}</Badge>
            <span class="text-xs text-[var(--muted)]">{{ b.time }}</span>
          </div>
          <p class="mt-1 truncate text-xs text-[var(--muted)]" :title="b.file">{{ b.file }}</p>
          <div class="mt-2 flex flex-wrap items-center gap-1.5">
            <Button variant="gray" size="sm" :disabled="verifyingFile === b.file" @click="verifyBak(b)">
              <ShieldCheck class="h-3.5 w-3.5" /> {{ verifyingFile === b.file ? "校验中…" : "校验" }}
            </Button>
            <Button variant="primary" size="sm" :disabled="restoring" @click="openRestoreBak(b)">
              <RotateCcw class="h-3.5 w-3.5" /> 恢复
            </Button>
          </div>
        </div>
      </div>

      <div class="mt-4 border-t border-[var(--card-border)] pt-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div class="min-w-0">
            <h4 class="text-sm font-semibold">软件配置档案</h4>
            <p class="mt-0.5 text-xs text-[var(--muted)]">勾选已安装软件的配置目录，备份为 config-时间.jwbak。未找到的路径不会被打包。</p>
          </div>
          <Button variant="gray" size="sm" :disabled="profilesLoading" @click="loadProfiles">
            <RefreshCw class="h-3.5 w-3.5" /> 刷新档案
          </Button>
        </div>

        <div v-if="profilesLoading" class="mt-3 text-xs text-[var(--muted)]">正在读取配置档案…</div>
        <div v-else-if="profiles.length === 0" class="mt-3 text-sm text-[var(--muted)]">没有读到配置档案。</div>
        <div v-else class="mt-3 max-h-72 space-y-1 overflow-y-auto pr-1">
          <label
            v-for="p in profiles"
            :key="p.id"
            class="flex cursor-pointer flex-wrap items-center gap-2 rounded-lg px-2 py-1.5 transition-colors hover:bg-[var(--btn-gray)]"
          >
            <Checkbox :checked="profileChecked.has(p.id)" @update:checked="(v) => setProfileChecked(p.id, !!v)" />
            <span class="truncate text-sm">{{ p.name }}</span>
            <Badge :tone="p.exists ? 'primary' : 'gray'">{{ p.exists ? "已找到" : "未找到" }}</Badge>
            <span class="min-w-0 w-full flex-1 truncate text-xs text-[var(--muted)] sm:w-auto" :title="p.paths.join(' | ')">
              {{ p.paths.join("；") }}
            </span>
          </label>
        </div>

        <div class="mt-3 flex flex-wrap items-center gap-2">
          <Input v-model="profileNote" placeholder="软件配置备份的备注，可留空" class="min-w-[12rem] flex-1" />
          <Button variant="primary" size="sm" :disabled="profileBacking || profileChecked.size === 0" @click="backupSelectedProfiles">
            <Save class="h-3.5 w-3.5" /> {{ profileBacking ? "备份中…" : "备份所选软件配置" }}
          </Button>
          <span class="text-xs text-[var(--muted)]">已勾选 {{ profileChecked.size }} 个</span>
        </div>
      </div>
    </Card>

    <!-- 生成备份清单：自定义名称 -->
    <Dialog :open="showCreate" title="生成备份清单" @update:open="showCreate = $event">
      <div class="space-y-3">
        <p class="text-sm text-[var(--muted)]">
          将把已勾选的 {{ appChecked.size }} 个软件（名称 / 版本 / 厂商 / 卸载入口）保存为备份清单。
        </p>
        <div>
          <label class="mb-1.5 block text-xs text-[var(--muted)]">备份名称</label>
          <Input v-model="createName" placeholder="例如：办公机常用软件" />
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-3">
          <Button variant="gray" @click="showCreate = false">取消</Button>
          <Button variant="primary" :disabled="saving" @click="confirmCreate">保存</Button>
        </div>
      </template>
    </Dialog>

    <!-- 重命名 -->
    <Dialog :open="showRename" title="重命名备份清单" @update:open="showRename = $event">
      <Input v-model="renameName" placeholder="输入新名称" />
      <template #footer>
        <div class="flex justify-end gap-3">
          <Button variant="gray" @click="showRename = false">取消</Button>
          <Button variant="primary" @click="confirmRename">确定</Button>
        </div>
      </template>
    </Dialog>

    <!-- 清单与本机对比 -->
    <Dialog :open="compareOpen" :title="`对比 · ${compareName}`" class="!max-w-2xl" @update:open="compareOpen = $event">
      <Spinner v-if="compareLoading" label="正在枚举本机软件并比对…" />
      <div v-else-if="compareResult" class="space-y-3">
        <div class="grid grid-cols-3 gap-2 text-center">
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-2.5">
            <p class="text-[10px] text-[var(--muted)]">清单内软件</p>
            <p class="text-xl font-bold tabular-nums">{{ compareResult.inList.length }}</p>
          </div>
          <div class="rounded-xl border border-[#30a46c]/40 bg-[#30a46c]/10 p-2.5">
            <p class="text-[10px] text-[var(--muted)]">本机已装</p>
            <p class="text-xl font-bold tabular-nums text-[#30a46c]">
              {{ compareResult.inList.length - compareResult.missing.length }}
            </p>
          </div>
          <div class="rounded-xl border border-[#e5484d]/40 bg-[#e5484d]/10 p-2.5">
            <p class="text-[10px] text-[var(--muted)]">清单里缺装</p>
            <p class="text-xl font-bold tabular-nums text-[#e5484d]">{{ compareResult.missing.length }}</p>
          </div>
        </div>

        <div v-if="compareResult.missing.length" class="rounded-xl border border-[var(--card-border)] p-3">
          <p class="mb-1.5 text-xs font-semibold text-[#e5484d]">清单里有、本机没装</p>
          <div class="max-h-48 overflow-y-auto">
            <p
              v-for="(m, i) in compareResult.missing"
              :key="`${m.name}-${i}`"
              class="truncate py-0.5 text-xs text-[var(--text)]"
              :title="`${m.name} ${m.version}`"
            >
              · {{ m.name }}<span v-if="m.version" class="ml-1 text-[var(--muted)]">{{ m.version }}</span>
            </p>
          </div>
        </div>
        <div v-else class="rounded-xl border border-[#30a46c]/40 bg-[#30a46c]/10 p-3 text-center text-xs text-[#30a46c]">
          清单里的软件本机都已安装
        </div>

        <div v-if="compareResult.extra.length" class="rounded-xl border border-[var(--card-border)] p-3">
          <p class="mb-1.5 text-xs font-semibold text-[var(--muted)]">
            本机有、清单里没有（{{ compareResult.extra.length }}，可能清单生成后新装）
          </p>
          <div class="max-h-32 overflow-y-auto">
            <p v-for="(e, i) in compareResult.extra" :key="`${e.name}-${i}`" class="truncate py-0.5 text-xs text-[var(--muted)]">
              · {{ e.name }}
            </p>
          </div>
        </div>

        <p class="text-[10px] text-[var(--muted)]">
          其中 {{ compareResult.inLibrary }} 个在软件库里有对应安装条目，可一键勾选补装。
        </p>
      </div>
      <template #footer>
        <div class="flex flex-wrap justify-end gap-3">
          <Button variant="gray" @click="compareOpen = false">关闭</Button>
          <Button
            variant="primary"
            :disabled="!compareResult || compareResult.missing.length === 0"
            @click="selectMissing"
          >
            勾选缺装软件
          </Button>
        </div>
      </template>
    </Dialog>

    <!-- 删除确认 -->
    <ConfirmDialog
      :open="showDelete"
      title="删除备份清单"
      :message="`确定要删除备份清单「${deleteTarget?.name ?? ''}」吗？该操作不可撤销。`"
      confirm-text="删除"
      danger
      @update:open="showDelete = $event"
      @confirm="confirmDelete"
    />

    <!-- 恢复 .jwbak：沿用确认框，策略在卡片上选择 -->
    <ConfirmDialog
      :open="showRestore"
      title="恢复配置备份"
      :message="`即将把备份「${baseName(restoreTarget?.file ?? '')}」恢复到工具箱配置目录，策略为「${strategyLabel}」。若包内清单与文件哈希不一致，会直接拒绝，不会先删除已有文件。确定继续？`"
      confirm-text="开始恢复"
      :danger="strategy === 'overwrite'"
      @update:open="showRestore = $event"
      @confirm="confirmRestoreBak"
    />
  </div>
</template>
