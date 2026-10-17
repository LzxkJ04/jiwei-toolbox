<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import Button from "../components/ui/Button.vue";
import {
  repairStart,
  onRepairProgress,
  onRepairDone,
  type RepairDoneItem,
  type RepairProgress,
  type RepairDone,
} from "../lib/tauri";

type Phase = "ready" | "running" | "done" | "error";

const phase = ref<Phase>("ready");
const pct = ref(0);
const stageMsg = ref("");
const items = ref<RepairDoneItem[]>([]);
const failMsg = ref("");
let unProg: (() => void) | null = null;
let unDone: (() => void) | null = null;

const STATUS_META: Record<string, { icon: string; cls: string }> = {
  ok: { icon: "✓", cls: "text-emerald-500" },
  repaired: { icon: "🔧", cls: "text-sky-500" },
  warn: { icon: "!", cls: "text-amber-500" },
};

function meta(status: string) {
  return STATUS_META[status] ?? { icon: "…", cls: "text-[var(--text-sub)]" };
}

function onProgress(p: RepairProgress) {
  pct.value = p.pct;
  stageMsg.value = p.msg;
}

function onDone(d: RepairDone) {
  items.value = d.items ?? [];
  if (d.ok) {
    phase.value = "done";
  } else {
    phase.value = "error";
    const warn = d.items?.find((i) => i.status === "warn");
    failMsg.value = warn?.detail ?? "修复未全部完成，请查看各项详情";
  }
  cleanup();
}

function cleanup() {
  unProg?.(); unProg = null;
  unDone?.(); unDone = null;
}

async function begin() {
  phase.value = "running";
  pct.value = 0;
  stageMsg.value = "准备中…";
  items.value = [];
  failMsg.value = "";
  unProg = await onRepairProgress(onProgress);
  unDone = await onRepairDone(onDone);
  const r = await repairStart();
  if (r === null) {
    phase.value = "error";
    failMsg.value = "无法启动修复流程";
    cleanup();
  }
}

function closeWindow() {
  window.close();
}

onBeforeUnmount(cleanup);
</script>

<template>
  <div class="flex h-screen items-center justify-center overflow-hidden px-6" style="background: var(--bg)">
    <div class="w-full max-w-[560px] rounded-2xl border border-[var(--border)] p-7 shadow-lg" style="background: var(--card)">
      <!-- 标题与说明 -->
      <div class="flex items-center gap-3">
        <div class="flex h-11 w-11 items-center justify-center rounded-2xl text-xl font-bold text-white" style="background: var(--primary)">修</div>
        <div>
          <h1 class="text-xl font-bold">修复安装</h1>
          <p class="mt-0.5 text-[12.5px] text-[var(--text-sub)]">
            将校验程序文件、重建目录配置、修复快捷方式与文件关联，不影响用户数据
          </p>
        </div>
      </div>

      <!-- 就绪 -->
      <template v-if="phase === 'ready'">
        <div class="mt-6 rounded-xl border border-[var(--border)] p-4 text-[13px] leading-6 text-[var(--text-sub)]" style="background: var(--bg)">
          修复过程包括：
          <div class="mt-1.5 ml-4 list-decimal">
            <li>校验程序文件完整性</li>
            <li>检查并重建目录配置（dirs.json）</li>
            <li>重建桌面/开始菜单快捷方式与文件关联</li>
          </div>
        </div>
        <Button variant="primary" class="mt-6 w-full" @click="begin">开始修复</Button>
      </template>

      <!-- 进行中 -->
      <template v-else-if="phase === 'running'">
        <div class="mt-7 text-center">
          <div class="text-5xl font-extrabold tabular-nums" style="color: var(--primary)">{{ Math.round(pct) }}%</div>
          <div class="mx-auto mt-4 h-3 max-w-sm overflow-hidden rounded-full bg-[var(--border)]">
            <div class="h-full rounded-full transition-[width]" :style="{ width: pct + '%', background: 'var(--primary)' }" />
          </div>
          <div class="mt-3 text-[13px] text-[var(--text-sub)]">{{ stageMsg || "正在修复…" }}</div>
        </div>
        <div class="mt-5 text-center text-[11px] text-[var(--muted)]">修复过程中请勿关闭窗口或结束进程</div>
      </template>

      <!-- 完成 -->
      <template v-else>
        <div class="mt-6 flex flex-col items-center">
          <div
            class="pop-in flex h-14 w-14 items-center justify-center rounded-full text-2xl text-white"
            :class="phase === 'done' ? 'bg-emerald-500 shadow-[0_10px_28px_rgba(22,163,74,.3)]' : 'bg-amber-500 shadow-[0_10px_28px_rgba(245,158,11,.3)]'"
          >
            {{ phase === "done" ? "✓" : "!" }}
          </div>
          <h2 class="mt-3 text-lg font-bold" :class="phase === 'done' ? 'text-emerald-500' : 'text-amber-500'">
            {{ phase === "done" ? "修复完成" : "修复未全部完成" }}
          </h2>
          <p v-if="phase === 'error'" class="mt-1 text-center text-[12.5px] text-[var(--text-sub)]">{{ failMsg }}</p>
        </div>
        <div class="mt-5 space-y-2">
          <div
            v-for="(it, i) in items"
            :key="i"
            class="flex items-start gap-3 rounded-xl border border-[var(--border)] px-4 py-3 text-[13px]"
            style="background: var(--bg)"
          >
            <span class="mt-0.5 font-bold" :class="meta(it.status).cls">{{ meta(it.status).icon }}</span>
            <div class="min-w-0">
              <div class="font-medium">{{ it.item }}</div>
              <div class="mt-0.5 break-all text-[12px] text-[var(--text-sub)]">{{ it.detail }}</div>
            </div>
          </div>
        </div>
        <Button variant="gray" class="mt-6 w-full" @click="closeWindow">关闭窗口</Button>
      </template>
    </div>
  </div>
</template>
