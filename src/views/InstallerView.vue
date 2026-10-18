<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import Button from "../components/ui/Button.vue";
import ConfirmDialog from "../components/ui/ConfirmDialog.vue";
import { relaunchPortable } from "../lib/tauri";
import {
  instChooseDir,
  instDiskFree,
  instStart,
  instFinish,
  instCancel,
  onInstallerProgress,
  onInstallerDone,
  instDetectExisting,
  installPrecheck,
  getEnvStatus as envStatus,
  type InstallerProgress,
  type InstallerDone,
  type ExistingInstall,
  type PrecheckItem,
} from "../lib/tauri";

type Step = 0 | 1 | 2 | 3 | 4 | 5 | 6;
const step = ref<Step>(0);
const stepNames = ["欢迎安装", "许可协议", "安装位置", "附加组件", "依赖预检", "正在安装", "安装完成"];

/* —— 协议 —— */
const agree = ref(false);
const LICENSE = `许可协议与免责声明：

1. 本软件按"现状"提供，仅供本地 Windows 系统维护使用。
2. 深度卸载、系统服务与注册表操作可能影响系统稳定性，请自行评估风险并提前备份。
3. 因不当使用本工具造成的损失，作者不承担责任。
4. 本软件所有数据均保存在本地，不收集、不上传任何用户数据。
5. 继续安装即表示你已阅读并接受以上全部条款。`;

/* —— 双路径 —— */
const programDir = ref("C:\\Program Files\\极维工具箱");
const dataDir = ref("C:\\Users\\Administrator\\AppData\\Local\\极维工具箱");
const progFree = ref(0);
const dataFree = ref(0);
const MIN_FREE = 500 * 1024 * 1024;

const sameDir = computed(() => {
  const a = programDir.value.trim().replace(/[\\/]+$/, "").toLowerCase();
  const b = dataDir.value.trim().replace(/[\\/]+$/, "").toLowerCase();
  return a.length > 3 && a === b;
});
const progOk = computed(() => progFree.value === 0 || progFree.value >= MIN_FREE);
const dataOk = computed(() => dataFree.value === 0 || dataFree.value >= MIN_FREE);
const dirOk = computed(() => progOk.value && dataOk.value && !sameDir.value && programDir.value.trim().length > 3);

/* —— 已有安装检测（版本升级预填） —— */
const existing = ref<ExistingInstall | null>(null);
onMounted(async () => {
  const d = await instDetectExisting();
  if (d?.exists) {
    existing.value = d;
    if (d.program.trim()) programDir.value = d.program;
    if (d.data.trim()) dataDir.value = d.data;
    refreshFree();
  }
});
const existingTip = computed(() =>
  existing.value
    ? `检测到已有安装${existing.value.oldVersion ? `（v${existing.value.oldVersion}）` : ""}，已预填原目录，程序数据将完整保留`
    : ""
);

function fmtGB(b: number) {
  return b <= 0 ? "未知" : (b / 1024 ** 3).toFixed(1) + " GB";
}
async function refreshFree() {
  progFree.value = (await instDiskFree(programDir.value.trim())) ?? 0;
  dataFree.value = (await instDiskFree(dataDir.value.trim())) ?? 0;
}
async function browse(target: "prog" | "data") {
  const d = await instChooseDir();
  if (d) {
    if (target === "prog") programDir.value = d;
    else dataDir.value = d;
  }
  refreshFree();
}

/* —— 组件 —— */
const desktop = ref(true);
const startMenu = ref(true);
const installWebv2 = ref(true);

/* —— 依赖预检 —— */
const hasWv2 = ref(true);
const wv2Checking = ref(true);
const wv2Pct = ref(0);
const wv2Msg = ref("");
let unWv2: (() => void) | null = null;
import { onWebview2Progress, downloadWebview2, installWebview2 } from "../lib/tauri";
const precheckDone = computed(() => wv2Checking.value === false && hasWv2.value);

/** 真实环境预检结果（系统版本 / 架构 / 权限 / 磁盘） */
const checks = ref<PrecheckItem[]>([]);
const checksLoading = ref(false);
/** 有 fail 项时禁止继续 */
const hasFail = computed(() => checks.value.some((c) => c.level === "fail"));

async function runChecks() {
  checksLoading.value = true;
  checks.value = (await installPrecheck(programDir.value.trim(), dataDir.value.trim())) ?? [];
  checksLoading.value = false;
}

function levelIcon(level: string) {
  if (level === "ok") return "✅";
  if (level === "warn") return "⚠️";
  return "❌";
}

function levelClass(level: string) {
  if (level === "ok") return "text-emerald-500";
  if (level === "warn") return "text-amber-500";
  return "text-red-500";
}

async function runPrecheck() {
  wv2Checking.value = true;
  void runChecks();
  try {
    const st = await envStatus();
    hasWv2.value = !!st?.has_webview2;
  } catch {
    hasWv2.value = true; // 检测失败按可用处理，安装时兜底
  }
  if (!hasWv2.value && installWebv2.value) {
    unWv2 = await onWebview2Progress((p) => {
      wv2Pct.value = p.pct;
      wv2Msg.value = p.stage === "download" ? "正在下载 WebView2 运行时…" : "正在安装 WebView2 运行时…";
      if (p.pct >= 100) hasWv2.value = true;
    });
    try {
      await downloadWebview2();
      await installWebview2();
      hasWv2.value = true;
    } catch {
      hasWv2.value = false;
    }
  }
  wv2Checking.value = false;
}

/* —— 安装进度 —— */
const pct = ref(0);
const pctShown = ref(0);
const stageMsg = ref("准备中…");
const logs = ref<string[]>([]);
const installing = ref(false);
const installDone = ref(false);
const installErr = ref("");
let unProg: (() => void) | null = null;
let unDone: (() => void) | null = null;
let animTimer: number | null = null;

async function beginInstall() {
  installing.value = true;
  installDone.value = false;
  installErr.value = "";
  logs.value = [];
  pct.value = 0;
  unProg = await onInstallerProgress((p: InstallerProgress) => {
    pct.value = p.pct;
    stageMsg.value = p.msg;
    if (p.stage === "done") {
      pct.value = 100;
    }
    if (p.msg && !logs.value.includes(p.msg)) logs.value.push(p.msg);
  });
  unDone = await onInstallerDone((d: InstallerDone) => {
    installing.value = false;
    if (d.ok) {
      installDone.value = true;
      pct.value = 100;
      stageMsg.value = "安装已完成";
    } else {
      installErr.value = d.err ?? "安装失败";
    }
    cleanup();
  });
  const r = await instStart(programDir.value, dataDir.value, desktop.value, startMenu.value, false, installWebv2.value);
  if (r === null) {
    // inst_start 返回 null 表示失败（tryCall 捕获 Err 为 null）
    installing.value = false;
    installErr.value = "无法启动安装进程";
  }
}
// 进度缓动动画
animTimer = window.setInterval(() => {
  pctShown.value += (pct.value - pctShown.value) * 0.12;
  if (Math.abs(pct.value - pctShown.value) < 0.5) pctShown.value = pct.value;
}, 60);

/* —— 完成 —— */
const runAfter = ref(true);
const openCfg = ref(false);

/* —— 取消 —— */
const showCancel = ref(false);
const showPortable = ref(false);
async function onCancel() {
  await instCancel();
  window.close();
}

function cleanup() {
  unProg?.(); unProg = null;
  unDone?.(); unDone = null;
  unWv2?.(); unWv2 = null;
}
onBeforeUnmount(() => {
  cleanup();
  if (animTimer) window.clearInterval(animTimer);
});

function next() {
  if (step.value === 2) refreshFree();
  if (step.value === 4) { /* 预检已自动跑 */ }
  step.value = (step.value + 1) as Step;
  if (step.value === 6) { /* noop */ }
  if (step.value === 5) beginInstall();
  if (step.value === 4) runPrecheck();
}
function prev() {
  step.value = Math.max(0, step.value - 1) as Step;
}
function canNext() {
  if (step.value === 0) return true;
  if (step.value === 1) return agree.value;
  if (step.value === 2) return dirOk.value;
  if (step.value === 4) return precheckDone.value && !hasFail.value;
  if (step.value === 5) return installDone.value;
  return true;
}
</script>

<template>
  <div class="flex h-screen overflow-hidden" style="background: var(--bg)">
    <!-- 左侧品牌栏 -->
    <aside class="relative flex w-[236px] shrink-0 flex-col items-center overflow-hidden px-5 pb-5 pt-11 text-white"
      style="background: linear-gradient(168deg, #182a54 0%, #24449c 55%, #2f6fed 100%)">
      <div class="absolute -left-16 -top-16 h-56 w-56 rounded-full"
        style="background: radial-gradient(circle, rgba(255,255,255,.14), transparent 65%)" />
      <div class="absolute -bottom-14 -right-14 h-48 w-48 rounded-full"
        style="background: radial-gradient(circle, rgba(96,165,250,.25), transparent 60%)" />
      <div class="z-10 flex h-[76px] w-[76px] items-center justify-center rounded-3xl bg-white text-4xl font-bold shadow-xl"
        style="color: var(--primary)">极</div>
      <h1 class="z-10 mt-3 text-lg font-semibold tracking-wide">极维工具箱</h1>
      <p class="z-10 mt-1 text-[11px] text-white/60">一站式 Windows 系统运维工具箱</p>
      <div class="z-10 mt-9 w-full">
        <div v-for="(n, i) in stepNames" :key="n"
          class="flex items-center gap-2.5 py-1.5" :class="i % 2 === 0 ? '' : 'pl-3 opacity-0 h-0 py-0'">
        </div>
        <!-- 分组指示 -->
        <div v-for="(g, gi) in [
            { label: '欢迎与协议', range: [0, 1] },
            { label: '安装位置', range: [2, 2] },
            { label: '组件与预检', range: [3, 4] },
            { label: '安装', range: [5, 6] },
          ]" :key="gi" class="flex items-center gap-2.5 py-2">
          <div class="flex h-6 w-6 items-center justify-center rounded-full text-[11px] font-semibold transition-all"
            :class="step >= g.range[0] ? 'bg-white text-[#2f6fed] shadow-[0_0_0_4px_rgba(255,255,255,.16)]' : 'bg-white/15 text-white/70'">
            {{ gi + 1 }}
          </div>
          <span class="text-[13px]" :class="step >= g.range[0] ? 'font-semibold text-white' : 'text-white/55'">
            {{ g.label }}
          </span>
        </div>
      </div>
      <div class="z-10 mt-auto text-[10px] text-white/50">v0.1.0 · for Windows</div>
    </aside>

    <!-- 右侧内容 -->
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="min-h-0 min-w-0 flex-1 overflow-x-clip overflow-y-auto px-6 pb-3 pt-8 lg:px-9">
        <!-- 0 欢迎 -->
        <section v-if="step === 0" class="page-enter">
          <h2 class="text-2xl font-bold">欢迎安装极维工具箱</h2>
          <p class="mt-2 text-[13px] text-[var(--text-sub)]">
            批量装机 · 深度软件卸载 · 硬件检测 · 系统维护，面向装机、二手验机与系统重装运维人员。
          </p>
          <div class="mt-6 flex items-center justify-center rounded-2xl border border-[var(--border)] py-10"
            style="background: var(--card)">
            <svg width="220" height="130" viewBox="0 0 220 130" fill="none">
              <rect x="60" y="10" width="100" height="70" rx="8" stroke="var(--primary)" stroke-width="3" fill="none" />
              <rect x="70" y="20" width="80" height="44" rx="4" fill="var(--primary)" opacity=".12" />
              <rect x="45" y="80" width="130" height="8" rx="4" fill="var(--primary)" opacity=".5" />
              <circle cx="45" cy="38" r="14" fill="#22c55e" opacity=".9" />
              <path d="M39 38l4 4 8-8" stroke="#fff" stroke-width="2.5" fill="none" stroke-linecap="round" />
              <rect x="152" y="52" width="34" height="26" rx="5" fill="#f59e0b" opacity=".9" transform="rotate(-12 152 52)" />
              <circle cx="176" cy="18" r="10" fill="var(--primary)" opacity=".85" />
              <path d="M176 13v10M171 18h10" stroke="#fff" stroke-width="2" stroke-linecap="round" />
            </svg>
          </div>
          <div class="mt-4 text-center">
            <button class="text-[13px] text-[var(--text-sub)] underline-offset-4 transition-colors hover:text-[var(--primary)] hover:underline"
              @click="showPortable = true">
              不安装，直接以便携模式运行
            </button>
          </div>
        </section>

        <!-- 1 协议 -->
        <section v-else-if="step === 1" class="page-enter flex h-full flex-col">
          <h2 class="text-2xl font-bold">许可协议</h2>
          <div class="mt-4 min-h-0 flex-1 overflow-y-auto rounded-2xl border border-[var(--border)] p-5 text-[13px] leading-7 whitespace-pre-line"
            style="background: var(--card); color: var(--text-sub)">{{ LICENSE }}</div>
          <label class="mt-4 flex cursor-pointer items-center gap-2.5 text-[13.5px]">
            <input v-model="agree" type="checkbox" class="h-4 w-4 accent-[var(--primary)]" />
            我已阅读并同意许可协议
          </label>
        </section>

        <!-- 2 双路径 -->
        <section v-else-if="step === 2" class="page-enter">
          <h2 class="text-2xl font-bold">选择安装位置</h2>
          <p class="mt-2 text-[13px] text-[var(--text-sub)]">
            分别设置程序本体目录与用户数据目录，两者建议分开存放，重装程序不影响个人数据。
          </p>
          <div
            v-if="existingTip"
            class="mt-4 flex items-center gap-2.5 rounded-xl border border-sky-400/40 bg-sky-500/10 px-4 py-2.5 text-[13px] text-sky-500"
          >
            <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" /><path d="M12 16v-4M12 8h.01" />
            </svg>
            {{ existingTip }}
          </div>
          <div class="mt-5 space-y-5">
            <div>
              <div class="mb-1.5 text-[13px] font-medium">程序本体目录 <span class="text-[11px] text-[var(--text-sub)]">（存放主程序与内置资源）</span></div>
              <div class="flex gap-2.5">
                <input v-model="programDir" @change="refreshFree" spellcheck="false"
                  class="input flex-1" />
                <Button variant="gray" @click="browse('prog')">浏览…</Button>
              </div>
              <div class="mt-1 text-xs" :class="progOk ? 'text-emerald-500' : 'text-red-500'">
                目标磁盘剩余空间：{{ fmtGB(progFree) }}{{ progOk ? "" : " —— 磁盘空间不足，请更换目录" }}
              </div>
            </div>
            <div>
              <div class="mb-1.5 text-[13px] font-medium">用户数据目录 <span class="text-[11px] text-[var(--text-sub)]">（缓存、软件清单、日志、配置）</span></div>
              <div class="flex gap-2.5">
                <input v-model="dataDir" @change="refreshFree" spellcheck="false" class="input flex-1" />
                <Button variant="gray" @click="browse('data')">浏览…</Button>
              </div>
              <div class="mt-1 text-xs" :class="dataOk ? 'text-emerald-500' : 'text-red-500'">
                目标磁盘剩余空间：{{ fmtGB(dataFree) }}{{ dataOk ? "" : " —— 磁盘空间不足，请更换目录" }}
              </div>
            </div>
            <div v-if="sameDir" class="rounded-xl border border-red-400/40 bg-red-500/10 px-4 py-2.5 text-[13px] text-red-500">
              程序目录与用户数据目录不能相同，请修改后继续。
            </div>
          </div>
        </section>

        <!-- 3 组件 -->
        <section v-else-if="step === 3" class="page-enter">
          <h2 class="text-2xl font-bold">选择附加组件</h2>
          <div class="mt-5 space-y-3">
            <label class="card flex cursor-pointer items-center gap-3 p-4 text-sm transition-transform hover:-translate-y-0.5">
              <input v-model="desktop" type="checkbox" class="h-4 w-4 accent-[var(--primary)]" />
              创建桌面快捷方式
            </label>
            <label class="card flex cursor-pointer items-center gap-3 p-4 text-sm transition-transform hover:-translate-y-0.5">
              <input v-model="startMenu" type="checkbox" class="h-4 w-4 accent-[var(--primary)]" />
              添加到开始菜单
            </label>
            <label class="card flex items-center gap-3 p-4 text-sm opacity-80">
              <input v-model="installWebv2" type="checkbox" disabled class="h-4 w-4 accent-[var(--primary)]" />
              <span>
                自动检测并安装 WebView2 运行时
                <span class="ml-1 text-xs text-[var(--text-sub)]">（程序运行必需依赖，不可取消）</span>
              </span>
            </label>
          </div>
        </section>

        <!-- 4 预检 -->
        <section v-else-if="step === 4" class="page-enter">
          <h2 class="text-2xl font-bold">系统依赖预检</h2>
          <p class="mt-2 text-[13px] text-[var(--text-sub)]">
            逐项检查安装环境，全部通过后才能继续。带 ❌ 的项必须处理。
          </p>
          <div class="mt-5 space-y-3">
            <div v-if="checksLoading" class="card flex items-center gap-3 p-4 text-sm">
              <span class="animate-pulse">⏳</span> 正在检查系统环境…
            </div>
            <div v-for="c in checks" :key="c.id" class="card p-4 text-sm">
              <div class="flex flex-wrap items-center gap-3">
                <span :class="levelClass(c.level)">{{ levelIcon(c.level) }}</span>
                <span class="font-medium">{{ c.label }}</span>
                <span class="text-xs" :class="levelClass(c.level)">{{ c.detail }}</span>
              </div>
              <p v-if="c.hint" class="mt-1.5 pl-7 text-xs text-[var(--text-sub)]">{{ c.hint }}</p>
            </div>

            <div class="card p-4 text-sm">
              <div class="flex items-center gap-3">
                <span v-if="hasWv2" class="text-emerald-500">✅</span>
                <span v-else-if="wv2Checking" class="animate-pulse">⏳</span>
                <span v-else class="text-red-500">❌</span>
                WebView2 运行时
                <span v-if="hasWv2" class="text-xs text-[var(--text-sub)]">已就绪</span>
                <span v-else-if="wv2Checking" class="text-xs text-[var(--text-sub)]">{{ wv2Msg || "检测中…" }}</span>
                <span v-else class="text-xs text-red-500">检测/安装失败，可返回重试或手动安装后继续</span>
              </div>
              <div v-if="!hasWv2 && wv2Checking" class="mt-3 h-2 overflow-hidden rounded-full bg-[var(--border)]">
                <div class="h-full rounded-full transition-all" :style="{ width: wv2Pct + '%', background: 'var(--primary)' }" />
              </div>
            </div>

            <div class="flex justify-end">
              <Button variant="gray" size="sm" :disabled="checksLoading || wv2Checking" @click="runChecks">
                重新检测
              </Button>
            </div>
          </div>
        </section>

        <!-- 5 进度 -->
        <section v-else-if="step === 5" class="page-enter flex h-full flex-col">
          <h2 class="text-2xl font-bold">正在安装极维工具箱</h2>
          <div class="mt-8 text-center">
            <div class="text-6xl font-extrabold tabular-nums" style="color: var(--primary)">{{ Math.round(pctShown) }}%</div>
            <div class="mx-auto mt-5 h-4 max-w-xl overflow-hidden rounded-full bg-[var(--border)]">
              <div class="pfill h-full rounded-full transition-[width]" :style="{ width: pctShown + '%' }" />
            </div>
            <div class="mt-4 text-[13px] text-[var(--text-sub)]">{{ installErr ? "安装失败：" + installErr : stageMsg }}</div>
          </div>
          <div class="mt-auto max-h-32 min-h-[72px] overflow-y-auto rounded-xl p-3 font-mono text-[11px] leading-5"
            style="background: var(--card); color: var(--text-sub)">
            <div v-for="(l, i) in logs" :key="i">{{ l }}</div>
            <div v-if="!logs.length" class="opacity-60">等待安装动作…</div>
          </div>
        </section>

        <!-- 6 完成 -->
        <section v-else class="page-enter">
          <div class="flex flex-col items-center pt-6">
            <div class="pop-in flex h-20 w-20 items-center justify-center rounded-full bg-emerald-500 text-4xl text-white shadow-[0_12px_32px_rgba(22,163,74,.35)]">✓</div>
            <h2 class="mt-4 text-2xl font-bold text-emerald-500">安装成功！</h2>
            <p class="mt-2 text-[13px] text-[var(--text-sub)]">极维工具箱已安装到您的电脑。</p>
          </div>
          <div class="mt-6 space-y-2">
            <label class="flex cursor-pointer items-center gap-2.5 text-sm">
              <input v-model="runAfter" type="checkbox" class="h-4 w-4 accent-[var(--primary)]" /> 立即启动极维工具箱
            </label>
            <label class="flex cursor-pointer items-center gap-2.5 text-sm">
              <input v-model="openCfg" type="checkbox" class="h-4 w-4 accent-[var(--primary)]" /> 打开用户数据目录
            </label>
          </div>
        </section>
      </div>

      <!-- 底部按钮栏 -->
      <div class="flex h-16 shrink-0 items-center gap-3 border-t border-[var(--border)] px-6" style="background: var(--card)">
        <Button variant="gray" @click="showCancel = true">取消</Button>
        <div class="flex-1" />
        <Button variant="gray" :disabled="step === 0 || step === 5" @click="prev">上一步</Button>
        <Button variant="primary" :disabled="!canNext()" @click="step === 6 ? instFinish(runAfter, openCfg) : next()">
          {{ step === 5 ? (installDone ? "下一步" : "安装中…") : step === 6 ? "完成" : "下一步" }}
        </Button>
      </div>
    </div>

    <ConfirmDialog :open="showPortable" title="便携模式"
      message="便携模式不写注册表、不创建快捷方式，直接以当前位置运行主程序。确定继续？"
      confirm-text="以便携模式运行" @update:open="showPortable = $event" @confirm="relaunchPortable()" />

    <ConfirmDialog :open="showCancel" title="退出安装" message="确定退出安装？已复制的文件将会自动清理回滚。"
      confirm-text="确认退出" danger @update:open="showCancel = $event" @confirm="onCancel" />
  </div>
</template>
