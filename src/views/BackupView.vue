<script setup lang="ts">
// 清单备份：后端持久化（data\备份导出\*.jwinv.json）；导出/导入 JSON 文件用于跨电脑迁移。
import { computed, onMounted, ref } from "vue";
import { Download, FileUp, GitCompare, ListRestart, Pencil, RefreshCw, Save, Trash2 } from "lucide-vue-next";
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
import {
  getAutoBackupInventory,
  listInstalled,
  setAutoBackupInventory,
  saveInventory,
  listInventories,
  loadInventoryDoc,
  deleteInventory,
  renameInventory,
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

onMounted(async () => {
  void scanInstalled();
  refreshInventories();
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

    <!-- 生成备份清单：自定义名称 -->
    <Dialog :open="showCreate" title="生成备份清单" @update:open="showCreate = $event">
      <div class="space-y-3">
        <p class="text-sm text-[var(--muted)]">
          将把已勾选的 {{ appChecked.size }} 个软件（名称 / 版本 / 厂商 / 卸载入口）保存为备份清单。
        </p>
        <div>
          <label class="mb-1.5 block text-xs text-[var(--muted)]">备份名称</label>
          <Input v-model="createName" placeholder="例如：办公机常用软件" @keydown.enter="confirmCreate" />
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
      <Input v-model="renameName" placeholder="输入新名称" @keydown.enter="confirmRename" />
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
  </div>
</template>
