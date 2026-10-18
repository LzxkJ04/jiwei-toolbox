<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  AlertTriangle, CheckCircle2, ExternalLink, Loader2, RefreshCw, Trash2,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import {
  deleteShortcuts, openPath, revealPath, scanShortcuts,
  type ShortcutRow,
} from "../lib/tauri";
import { formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

const rows = ref<ShortcutRow[] | null>(null);
const scanning = ref(false);
const busy = ref(false);
const onlyBroken = ref(true);
const checked = ref<Set<string>>(new Set());
const showConfirm = ref(false);

const broken = computed(() => (rows.value ?? []).filter((r) => r.broken));
const total = computed(() => rows.value?.length ?? 0);

const shown = computed(() => {
  const all = rows.value ?? [];
  if (!onlyBroken.value) return all;
  return all.filter((r) => r.broken);
});

/** 按位置分组，界面按组展示更容易判断 */
const groups = computed(() => {
  const map = new Map<string, ShortcutRow[]>();
  for (const r of shown.value) {
    const list = map.get(r.location) ?? [];
    list.push(r);
    map.set(r.location, list);
  }
  return [...map.entries()].map(([location, items]) => ({ location, items }));
});

const reclaimable = computed(() =>
  broken.value.reduce((s, r) => s + r.size, 0)
);

function toggle(path: string) {
  const next = new Set(checked.value);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  checked.value = next;
}

function toggleAllBroken() {
  if (checked.value.size === broken.value.length) {
    checked.value = new Set();
  } else {
    checked.value = new Set(broken.value.map((r) => r.path));
  }
}

async function scan() {
  scanning.value = true;
  checked.value = new Set();
  try {
    rows.value = (await scanShortcuts()) ?? [];
    const n = rows.value.filter((r) => r.broken).length;
    toast(n > 0 ? `扫描完成，发现 ${n} 个失效快捷方式` : "扫描完成，没有发现失效快捷方式",
      n > 0 ? "warning" : "success");
  } catch (err) {
    toast(`扫描失败：${String(err)}`, "error");
  } finally {
    scanning.value = false;
  }
}

async function doDelete() {
  const paths = [...checked.value];
  if (paths.length === 0) return;
  busy.value = true;
  try {
    const n = await deleteShortcuts(paths);
    toast(`已删除 ${n} 个快捷方式（可从回收站还原）`, "success");
    // 直接从本地列表移除，避免整轮重扫
    const gone = new Set(paths);
    rows.value = (rows.value ?? []).filter((r) => !gone.has(r.path));
    checked.value = new Set();
  } catch (err) {
    toast(`删除失败：${String(err)}`, "error");
  } finally {
    busy.value = false;
  }
}

async function reveal(p: string) {
  try {
    await revealPath(p);
  } catch (err) {
    toast(String(err), "error");
  }
}

async function openTarget(r: ShortcutRow) {
  if (!r.target) {
    toast("这条快捷方式没有可解析的目标", "info");
    return;
  }
  const ok = await openPath(r.target);
  if (ok === null) toast("目标不存在，无法打开", "warning");
}

onMounted(scan);
</script>

<template>
  <div>
    <PageHeader title="失效快捷方式" desc="扫描桌面与开始菜单，找出指向已删除程序的快捷方式。">
      <template #extra>
        <Button variant="gray" size="sm" :disabled="scanning || busy" @click="scan">
          <RefreshCw class="h-3.5 w-3.5" :class="scanning && 'animate-spin'" />
          重新扫描
        </Button>
      </template>
    </PageHeader>

    <!-- 汇总 -->
    <div class="mb-4 grid gap-3 sm:grid-cols-3">
      <Card class="p-4">
        <p class="text-xs text-[var(--muted)]">扫描到的快捷方式</p>
        <p class="mt-1 text-2xl font-bold tabular-nums">{{ total }}</p>
      </Card>
      <Card class="p-4">
        <p class="text-xs text-[var(--muted)]">失效</p>
        <p class="mt-1 text-2xl font-bold tabular-nums" :class="broken.length > 0 ? 'text-[#e5484d]' : 'text-[#30a46c]'">
          {{ broken.length }}
        </p>
      </Card>
      <Card class="p-4">
        <p class="text-xs text-[var(--muted)]">可回收空间</p>
        <p class="mt-1 text-2xl font-bold tabular-nums">{{ formatBytes(reclaimable) }}</p>
      </Card>
    </div>

    <Card class="p-5">
      <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h4 class="flex items-center gap-2 font-semibold">
          快捷方式列表
          <Badge v-if="broken.length > 0" tone="danger">{{ broken.length }} 个失效</Badge>
          <Badge v-else-if="rows !== null" tone="success">
            <CheckCircle2 class="h-3 w-3" /> 全部正常
          </Badge>
        </h4>
        <div class="flex flex-wrap items-center gap-2">
          <label class="flex cursor-pointer items-center gap-2 text-xs text-[var(--muted)]">
            <input v-model="onlyBroken" type="checkbox" class="accent-[var(--primary)]" />
            只看失效
          </label>
          <Button
            v-if="broken.length > 0"
            variant="gray"
            size="sm"
            @click="toggleAllBroken"
          >
            {{ checked.size === broken.length ? "取消全选" : `全选失效（${broken.length}）` }}
          </Button>
          <Button
            variant="danger"
            size="sm"
            :disabled="checked.size === 0 || busy"
            @click="showConfirm = true"
          >
            <Loader2 v-if="busy" class="h-3.5 w-3.5 animate-spin" />
            <Trash2 v-else class="h-3.5 w-3.5" />
            删除已选（{{ checked.size }}）
          </Button>
        </div>
      </div>

      <Spinner v-if="scanning && rows === null" label="正在扫描桌面与开始菜单…" />

      <EmptyState
        v-else-if="rows !== null && shown.length === 0"
        icon="✨"
        :text="onlyBroken ? '没有发现失效快捷方式' : '没有扫描到快捷方式'"
        sub="取消「只看失效」可以查看全部快捷方式"
      />

      <div v-else class="space-y-4">
        <div v-for="g in groups" :key="g.location">
          <p class="mb-1.5 text-xs font-medium text-[var(--muted)]">
            {{ g.location }} · {{ g.items.length }} 个
          </p>
          <div class="divide-y divide-[var(--card-border)] rounded-xl border border-[var(--card-border)]">
            <div
              v-for="r in g.items"
              :key="r.path"
              class="flex items-start gap-2.5 px-3 py-2 transition-colors hover:bg-[var(--card-soft)]"
              :class="r.broken && 'bg-[#e5484d08]'"
            >
              <input
                v-if="r.broken"
                type="checkbox"
                class="mt-1 shrink-0 accent-[var(--primary)]"
                :checked="checked.has(r.path)"
                @change="toggle(r.path)"
              />
              <span v-else class="mt-1 h-3.5 w-3.5 shrink-0" />
              <div class="min-w-0 flex-1">
                <p class="flex flex-wrap items-center gap-1.5 text-sm">
                  <span class="truncate">{{ r.name }}</span>
                  <Badge v-if="r.broken" tone="danger" class="!px-1.5 !py-0 !text-[10px]">
                    <AlertTriangle class="h-2.5 w-2.5" /> {{ r.reason || "失效" }}
                  </Badge>
                </p>
                <p class="mt-0.5 truncate font-mono text-[10px] text-[var(--muted)]" :title="r.target || r.path">
                  {{ r.target || "（无目标，可能是商店应用）" }}
                </p>
              </div>
              <div class="flex shrink-0 gap-1">
                <Button v-if="r.target" variant="ghost" size="sm" title="打开目标" @click="openTarget(r)">
                  <ExternalLink class="h-3 w-3" />
                </Button>
                <Button variant="ghost" size="sm" title="在资源管理器中定位" @click="reveal(r.path)">
                  <ExternalLink class="h-3 w-3" />
                </Button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <p class="mt-3 text-[11px] leading-relaxed text-[var(--muted)]">
        判定方式：解析 .lnk 的目标路径，目标文件不存在即视为失效。
        目标为空的快捷方式（多为商店应用）不会被判定失效；网络路径也不做连通性检测，避免离线时成片误报。
        删除会进回收站，可以还原。
      </p>
    </Card>

    <ConfirmDialog
      :open="showConfirm"
      title="删除失效快捷方式"
      :message="`将删除 ${checked.size} 个快捷方式，释放约 ${formatBytes(
        broken.filter((r) => checked.has(r.path)).reduce((s, r) => s + r.size, 0)
      )}。\n\n删除的文件会进入回收站，可以还原。只删除 .lnk 文件本身，不会碰目标程序。`"
      confirm-text="删除"
      danger
      @update:open="(v) => !v && (showConfirm = false)"
      @confirm="doDelete"
    />
  </div>
</template>
