<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  CheckCircle2, FileText, FolderOpen, Loader2, RefreshCw, Save,
} from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Spinner from "../components/ui/Spinner.vue";
import {
  chooseDir, exportSystemReport, hwInfo, licenseStatus, openPath, revealPath,
  type HwInfo, type LicenseStatus, type ReportResult,
} from "../lib/tauri";
import { formatBytes } from "../lib/utils";
import { toast } from "../lib/toast";

const hw = ref<HwInfo | null>(null);
const lic = ref<LicenseStatus | null>(null);
const loading = ref(false);
const exporting = ref(false);
const target = ref("");
const last = ref<ReportResult | null>(null);

const cpu = computed(() => hw.value?.cpu);
const os = computed(() => hw.value?.os);
const memTotal = computed(() => hw.value?.memory_total ?? 0);

const gpuLine = computed(() => {
  const list = (hw.value?.gpus ?? []).filter((g) => !g.virtual);
  return list.length ? list.map((g) => g.name).join(" / ") : "—";
});

const diskLine = computed(() => {
  const list = hw.value?.disks ?? [];
  return list.length ? list.map((d) => `${d.model} ${formatBytes(d.size)}`).join(" / ") : "—";
});

/** 报告将包含的章节，让用户导出前知道内容 */
const SECTIONS = [
  { title: "整机摘要", desc: "CPU、内存、显卡、磁盘、授权状态" },
  { title: "处理器", desc: "型号、厂商、架构、插槽、缓存、虚拟化" },
  { title: "内存", desc: "总容量与每条内存的类型、频率、插槽" },
  { title: "显卡", desc: "型号、显存、驱动，虚拟显示器单独标注" },
  { title: "磁盘", desc: "型号、容量、接口、健康、序列号、分区占用" },
  { title: "主板与 BIOS", desc: "型号、序列号、BIOS 厂商与日期" },
  { title: "已安装软件", desc: "全部桌面软件名称、版本、发行商" },
];

async function load() {
  loading.value = true;
  const [h, l] = await Promise.all([hwInfo(), licenseStatus()]);
  hw.value = h;
  lic.value = l;
  loading.value = false;
}

async function pickDir() {
  const dir = await chooseDir("选择报告保存目录");
  if (dir) target.value = dir;
}

async function doExport() {
  exporting.value = true;
  try {
    const res = await exportSystemReport(target.value.trim() || undefined);
    if (res) {
      last.value = res;
      toast(`报告已导出：${res.app_count} 个软件，${formatBytes(res.size)}`, "success");
    }
  } catch (err) {
    toast(`导出失败：${String(err)}`, "error");
  } finally {
    exporting.value = false;
  }
}

async function openDir() {
  const f = last.value?.txt_file;
  if (!f) return;
  try {
    await revealPath(f);
  } catch (err) {
    toast(String(err), "error");
  }
}

async function openFile() {
  const f = last.value?.txt_file;
  if (!f) return;
  const ok = await openPath(f);
  if (ok === null) toast("无法打开文件", "error");
}

onMounted(load);
</script>

<template>
  <div>
    <PageHeader title="系统信息导出" desc="把硬件、系统、授权和已装软件汇总成一份报告，方便交付或求助。">
      <template #extra>
        <Button variant="gray" size="sm" :disabled="loading" @click="load">
          <RefreshCw class="h-3.5 w-3.5" :class="loading && 'animate-spin'" /> 重新读取
        </Button>
      </template>
    </PageHeader>

    <Spinner v-if="loading && !hw" label="正在读取系统信息…" />

    <template v-else>
      <!-- 预览 -->
      <Card class="mb-4 p-5">
        <h4 class="mb-3 font-semibold">本机概览</h4>
        <div class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
          <div class="flex min-w-0 gap-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">处理器</span>
            <span class="min-w-0 truncate font-medium" :title="cpu?.name">{{ cpu?.name || "—" }}</span>
          </div>
          <div class="flex min-w-0 gap-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">核心线程</span>
            <span class="font-medium">{{ cpu?.cores ?? 0 }} 核 {{ cpu?.threads ?? 0 }} 线程</span>
          </div>
          <div class="flex min-w-0 gap-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">内存</span>
            <span class="font-medium">{{ formatBytes(memTotal) }}</span>
          </div>
          <div class="flex min-w-0 gap-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">操作系统</span>
            <span class="min-w-0 truncate font-medium" :title="os?.caption">
              {{ os?.caption || lic?.osEdition || "—" }}
              <span v-if="os?.build" class="text-xs text-[var(--muted)]">Build {{ os.build }}</span>
            </span>
          </div>
          <div class="flex min-w-0 gap-2 sm:col-span-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">显卡</span>
            <span class="min-w-0 truncate font-medium" :title="gpuLine">{{ gpuLine }}</span>
          </div>
          <div class="flex min-w-0 gap-2 sm:col-span-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">磁盘</span>
            <span class="min-w-0 truncate font-medium" :title="diskLine">{{ diskLine }}</span>
          </div>
          <div class="flex items-center gap-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">授权</span>
            <Badge :tone="lic?.licensed ? 'success' : 'warning'">
              {{ lic?.statusText || "未知" }}
            </Badge>
          </div>
          <div class="flex min-w-0 gap-2">
            <span class="w-20 shrink-0 text-[var(--muted)]">主板</span>
            <span class="min-w-0 truncate font-medium" :title="hw?.board">{{ hw?.board || "—" }}</span>
          </div>
        </div>
      </Card>

      <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,320px)]">
        <!-- 报告章节 -->
        <Card class="p-5">
          <h4 class="mb-3 font-semibold">报告内容</h4>
          <div class="space-y-2">
            <div
              v-for="(s, i) in SECTIONS"
              :key="s.title"
              class="flex items-start gap-3 rounded-xl border border-[var(--card-border)] p-3"
            >
              <CheckCircle2 class="mt-0.5 h-4 w-4 shrink-0 text-[#30a46c]" />
              <div class="min-w-0">
                <p class="text-sm font-medium">{{ i + 1 }}. {{ s.title }}</p>
                <p class="mt-0.5 text-xs text-[var(--muted)]">{{ s.desc }}</p>
              </div>
            </div>
          </div>
          <p class="mt-3 text-[11px] leading-relaxed text-[var(--muted)]">
            同时导出两份文件：txt 给人工阅读，md 方便粘贴到工单或聊天工具。
            报告只读取本机信息，不会上传到任何服务器。
          </p>
        </Card>

        <!-- 导出操作 -->
        <div class="space-y-4">
          <Card class="p-5">
            <h4 class="mb-3 font-semibold">导出</h4>
            <p class="mb-1 text-xs text-[var(--muted)]">保存目录</p>
            <div class="flex gap-2">
              <input
                v-model="target"
                type="text"
                class="input-base min-w-0 flex-1 text-xs"
                placeholder="留空则保存到备份导出目录"
              />
              <Button variant="gray" size="sm" @click="pickDir">
                <FolderOpen class="h-3.5 w-3.5" />
              </Button>
            </div>
            <Button variant="primary" class="mt-3 w-full" :disabled="exporting" @click="doExport">
              <Loader2 v-if="exporting" class="h-4 w-4 animate-spin" />
              <Save v-else class="h-4 w-4" />
              导出报告
            </Button>
          </Card>

          <Card v-if="last" class="p-5">
            <h4 class="mb-2 flex items-center gap-2 font-semibold">
              <FileText class="h-4 w-4 text-[#30a46c]" /> 上次导出
            </h4>
            <div class="space-y-1.5 text-xs">
              <p class="text-[var(--muted)]">
                {{ last.app_count }} 个软件 · {{ formatBytes(last.size) }}
              </p>
              <p class="break-all font-mono text-[10px] text-[var(--muted)]">{{ last.txt_file }}</p>
              <p class="break-all font-mono text-[10px] text-[var(--muted)]">{{ last.md_file }}</p>
            </div>
            <div class="mt-3 flex gap-2">
              <Button variant="gray" size="sm" class="flex-1" @click="openFile">打开 txt</Button>
              <Button variant="gray" size="sm" class="flex-1" @click="openDir">所在目录</Button>
            </div>
          </Card>
        </div>
      </div>
    </template>
  </div>
</template>
