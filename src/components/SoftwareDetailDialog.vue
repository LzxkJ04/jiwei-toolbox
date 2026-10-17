<script setup lang="ts">
import { computed } from "vue";
import { CheckCircle2, Copy, Download, GitCommitHorizontal,
  Hash, Link2, ShieldCheck, Star, TriangleAlert, X, XCircle,
} from "lucide-vue-next";
import Dialog from "./ui/Dialog.vue";
import Button from "./ui/Button.vue";
import Badge from "./ui/Badge.vue";
import type { SoftwareItem } from "../lib/tauri";
import SoftwareIcon from "./SoftwareIcon.vue";
import { openPath } from "../lib/tauri";
import { useSoftwareStore } from "../stores/software";
import { useFavoritesStore } from "../stores/favorites";
import { useTasksStore } from "../stores/tasks";
import { copyText } from "../lib/utils";
import { toast } from "../lib/toast";

const props = defineProps<{ item: SoftwareItem | null; open: boolean }>();

const emit = defineEmits<{ (e: "update:open", v: boolean): void }>();

const store = useSoftwareStore();
const favorites = useFavoritesStore();
const tasks = useTasksStore();

const licenseLabel = computed(() => {
  if (!props.item) return "—";
  const lic = props.item.license;
  if (lic === "free" || lic === "open" || lic === "commercial") {
    return { free: "免费", open: "开源", commercial: "商业" }[lic];
  }
  return { free: "免费", open: "开源", commercial: "商业" }[store.licenseOf(props.item)];
});

/** 版本号：取更新日志最新一条，无则不显示 */
const versionLabel = computed(() => props.item?.changelog?.[0]?.version ?? "");

const homepageUrl = computed(() => props.item?.homepage || props.item?.url || "");

const installedApp = computed(() => (props.item ? store.installedOf(props.item) : null));
const isFav = computed(() => (props.item ? favorites.isFavorite(props.item.id) : false));

const silentSupported = computed(
  () => !!(props.item?.silentArgs && props.item.silentArgs.trim())
);

/** 依赖项名称映射（id → 清单名称） */
const dependsNames = computed(() => {
  if (!props.item?.depends?.length) return [];
  return props.item.depends.map(
    (id) => (store.items ?? []).find((s) => s.id === id)?.name ?? id
  );
});

const sizeLabel = computed(() => {
  const mb = props.item?.size_mb ?? 0;
  return mb > 0 ? `${mb} MB` : "--";
});

async function copy(value: string | undefined | null) {
  if (!value) return;
  const ok = await copyText(value);
  toast(ok ? "复制成功" : "复制失败", ok ? "success" : "error");
}

function openUrl() {
  if (homepageUrl.value) {
    try {
      window.open(homepageUrl.value, "_blank");
    } catch {
      /* ignore */
    }
  }
}

function toggleFav() {
  if (!props.item) return;
  void favorites.toggle(props.item.id).then((added) => {
    toast(added ? "已加入收藏" : "已取消收藏", "success");
  });
}

function addToQueue() {
  if (!props.item) return;
  const n = tasks.addFromSoftware([props.item]);
  toast(n > 0 ? "已加入任务队列" : "该软件已在任务队列中", n > 0 ? "success" : "info");
}

// TODO(后端化): 后端提供 open_path / 启动进程命令后，改为直接以 install_location 启动软件
async function openSoftware() {
  const loc = (installedApp.value as Record<string, unknown> | null)?.install_location as string | undefined;
  if (loc) {
    const ok = await openPath(loc);
    if (ok !== null) {
      toast("已在资源管理器中打开安装目录", "success");
      return;
    }
  }
  toast("未找到安装目录，请通过开始菜单或桌面快捷方式打开该软件", "warning");
}
</script>

<template>
  <Dialog
    :open="props.open && !!props.item"
    :title="undefined"
    class="!max-w-xl"
    @update:open="emit('update:open', $event)"
  >
    <div v-if="props.item" class="relative">
      <!-- 标题栏：图标 + 名称 + 版本 + 关闭 -->
      <div class="mb-4 flex items-start gap-3 pr-8">
        <SoftwareIcon :name="props.item?.name || ''" :homepage="props.item?.homepage" size="h-12 w-12" />
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <h3 class="truncate text-lg font-bold text-[var(--text-strong)]">
              {{ props.item.name || "未知软件" }}
            </h3>
            <span v-if="versionLabel" class="shrink-0 text-xs text-[var(--muted)]">v{{ versionLabel }}</span>
            <Badge tone="gray">{{ props.item.type || "exe" }}</Badge>
          </div>
          <p class="mt-0.5 text-xs text-[var(--muted)]">
            {{ props.item.category || "未分类" }}
          </p>
        </div>
      </div>
      <!-- X 关闭 -->
      <button
        class="absolute right-0 top-0 flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--btn-gray)] hover:text-[var(--text)]"
        title="关闭"
        @click="emit('update:open', false)"
      >
        <X class="h-4 w-4" />
      </button>

      <div class="space-y-3">
        <!-- 简介区：描述 + 授权标签 + 官网链接 -->
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3.5">
          <div class="mb-2 flex flex-wrap items-center gap-2">
            <Badge tone="primary">授权：{{ licenseLabel }}</Badge>
            <Badge v-if="!silentSupported" tone="danger">不支持静默安装</Badge>
          </div>
          <p class="text-sm leading-relaxed">{{ props.item.description || props.item.notes || "暂无简介。" }}</p>
          <button
            v-if="homepageUrl"
            class="mt-2 flex max-w-full cursor-pointer items-center gap-1.5 text-xs text-[var(--primary)] hover:underline"
            :title="homepageUrl"
            @click="openUrl"
          >
            <Link2 class="h-3.5 w-3.5 shrink-0" />
            <span class="truncate">{{ homepageUrl }}</span>
          </button>
        </div>

        <!-- 两列信息卡 -->
        <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
          <!-- 安装信息 -->
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3.5">
            <h4 class="mb-2 flex items-center gap-1.5 text-sm font-semibold">
              <Download class="h-4 w-4 text-[var(--primary)]" /> 安装信息
            </h4>
            <dl class="space-y-1.5 text-xs">
              <div class="flex items-center gap-2">
                <dt class="w-16 shrink-0 text-[var(--muted)]">大小</dt>
                <dd>{{ sizeLabel }}</dd>
              </div>
              <div class="flex items-center gap-2">
                <dt class="w-16 shrink-0 text-[var(--muted)]">SHA256</dt>
                <dd class="flex min-w-0 flex-1 items-center gap-1.5">
                  <Hash class="h-3.5 w-3.5 shrink-0 text-[var(--muted)]" />
                  <template v-if="props.item.sha256">
                    <span class="min-w-0 flex-1 truncate font-mono" :title="props.item.sha256">
                      {{ props.item.sha256 }}
                    </span>
                    <button
                      class="shrink-0 cursor-pointer text-[var(--muted)] transition-colors hover:text-[var(--primary)]"
                      title="复制 SHA256"
                      @click="copy(props.item.sha256)"
                    >
                      <Copy class="h-3.5 w-3.5" />
                    </button>
                  </template>
                  <span v-else class="text-[var(--muted)]">发布后校验</span>
                </dd>
              </div>
              <div class="flex items-center gap-2">
                <dt class="w-16 shrink-0 text-[var(--muted)]">静默支持</dt>
                <dd>
                  <span v-if="silentSupported" class="flex items-center gap-1 text-[#30a46c]">
                    <ShieldCheck class="h-3.5 w-3.5" /> 支持
                  </span>
                  <span v-else class="text-[#e5484d]">不支持</span>
                </dd>
              </div>
              <div class="flex items-start gap-2">
                <dt class="w-16 shrink-0 pt-0.5 text-[var(--muted)]">静默参数</dt>
                <dd class="flex min-w-0 flex-1 items-center gap-1.5">
                  <code
                    v-if="silentSupported"
                    class="min-w-0 flex-1 truncate rounded bg-[var(--btn-gray)] px-1.5 py-0.5 font-mono"
                    :title="props.item.silentArgs"
                  >
                    {{ props.item.silentArgs }}
                  </code>
                  <span v-else class="text-[var(--muted)]">—</span>
                  <button
                    v-if="silentSupported"
                    title="复制静默参数"
                    class="shrink-0 cursor-pointer text-[var(--muted)] transition-colors hover:text-[var(--primary)]"
                    @click="copy(props.item.silentArgs)"
                  >
                    <Copy class="h-3.5 w-3.5" />
                  </button>
                </dd>
              </div>
            </dl>
          </div>

          <!-- 注册表信息 -->
          <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3.5">
            <h4 class="mb-2 flex items-center justify-between gap-2 text-sm font-semibold">
              <span>注册表信息</span>
              <Button v-if="installedApp" variant="ghost" size="sm" title="打开该软件" @click="openSoftware">
                打开软件
              </Button>
            </h4>
            <template v-if="installedApp">
              <p class="flex items-start gap-1.5 text-xs">
                <CheckCircle2 class="mt-0.5 h-3.5 w-3.5 shrink-0 text-[#30a46c]" />
                <span>
                  已在本机注册：{{ installedApp.name }}
                  <span v-if="installedApp.version" class="text-[var(--muted)]">（v{{ installedApp.version }}）</span>
                </span>
              </p>
              <div class="mt-2 flex items-start gap-2">
                <code
                  class="min-w-0 flex-1 break-all rounded bg-[var(--btn-gray)] px-1.5 py-1 font-mono text-[10px] leading-relaxed"
                  :title="installedApp.key"
                >
                  {{ installedApp.key }}
                </code>
                <button
                  class="shrink-0 cursor-pointer text-[var(--muted)] transition-colors hover:text-[var(--primary)]"
                  title="复制注册表路径"
                  @click="copy(installedApp.key)"
                >
                  <Copy class="h-3.5 w-3.5" />
                </button>
              </div>
            </template>
            <p v-else class="flex items-center gap-1.5 text-xs text-[var(--muted)]">
              <XCircle class="h-3.5 w-3.5 shrink-0" />
              未在本机注册表中发现该软件
            </p>
          </div>
        </div>

        <!-- 依赖提示黄卡 -->
        <div
          v-if="dependsNames.length > 0"
          class="flex items-start gap-2 rounded-xl border border-[#f5a62355] bg-[#f5a6231a] p-3 text-xs leading-relaxed text-[#b57d10]"
        >
          <TriangleAlert class="mt-0.5 h-4 w-4 shrink-0" />
          <span>
            依赖提示：安装本软件前需先安装
            <b>{{ dependsNames.join("、") }}</b>
            ，请将依赖项一并加入任务队列。
          </span>
        </div>

        <!-- 更新日志滚动区 -->
        <div class="rounded-xl border border-[var(--card-border)] bg-[var(--card-soft)] p-3.5">
          <h4 class="mb-1.5 flex items-center gap-1.5 text-sm font-semibold">
            <GitCommitHorizontal class="h-4 w-4 text-[var(--primary)]" /> 更新日志
          </h4>
          <p v-if="!props.item.changelog?.length" class="text-xs text-[var(--muted)]">
            清单未提供更新日志
          </p>
          <div v-else class="max-h-44 space-y-2.5 overflow-y-auto pr-1">
            <div v-for="(entry, i) in props.item.changelog" :key="i" class="text-xs">
              <div class="flex items-center gap-2">
                <Badge tone="gray">{{ entry.version || "未知版本" }}</Badge>
                <span class="text-[var(--muted)]">{{ entry.date }}</span>
              </div>
              <p v-if="entry.notes" class="mt-1 leading-relaxed text-[var(--text)]">{{ entry.notes }}</p>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 底部操作：收藏 + 加入队列 + 关闭 -->
    <template #footer>
      <div v-if="props.item" class="flex items-center justify-between gap-3">
        <Button variant="gray" size="sm" @click="toggleFav">
          <Star
            class="h-4 w-4"
            :class="isFav && 'fill-[var(--primary)] text-[var(--primary)]'"
          />
          {{ isFav ? "已收藏" : "收藏" }}
        </Button>
        <div class="flex gap-2">
          <Button variant="gray" size="sm" @click="emit('update:open', false)">关闭</Button>
          <Button variant="primary" size="sm" @click="addToQueue">加入任务队列</Button>
        </div>
      </div>
    </template>
  </Dialog>
</template>
