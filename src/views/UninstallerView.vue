<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import Button from "../components/ui/Button.vue";
import { uninsStart, onUninstallerProgress, onUninstallerDone, getDirs } from "../lib/tauri";

type Phase = "confirm" | "running" | "done" | "error";
const phase = ref<Phase>("confirm");
const keepData = ref(true);
const dataDir = ref("");
const stageText = ref("准备卸载…");
const errText = ref("");

const STAGE_MAP: Record<string, string> = {
  stop: "正在结束相关进程…",
  files: "正在删除程序文件…",
  registry: "正在移除注册表项…",
  data: "正在处理用户数据…",
  done: "卸载完成",
};

(async () => {
  try {
    const d = (await getDirs()) as unknown as Record<string, string> | null;
    dataDir.value = d?.data || d?.cache || "";
  } catch { /* 静默 */ }
})();

async function start() {
  phase.value = "running";
  stageText.value = "正在结束相关进程…";
  const unP = await onUninstallerProgress((p) => {
    stageText.value = STAGE_MAP[p.stage] ?? p.stage;
  });
  const unD = await onUninstallerDone((d) => {
    unP();
    unD();
    if (d.ok) phase.value = "done";
    else {
      phase.value = "error";
      errText.value = "卸载程序返回失败，请检查是否有文件被占用后重试。";
    }
  });
  try {
    await uninsStart(keepData.value);
  } catch (e) {
    unP(); unD();
    phase.value = "error";
    errText.value = String(e) || "未找到卸载程序（unins000.exe）";
  }
}

function retry() {
  phase.value = "confirm";
}
function closeWin() {
  window.close();
}
onBeforeUnmount(() => { /* 事件句柄已在完成/失败时清理 */ });
</script>

<template>
  <div class="flex h-screen items-center justify-center overflow-hidden" style="background: var(--bg)">
    <div class="glass-card w-[460px] rounded-3xl p-8">
      <!-- 确认 -->
      <template v-if="phase === 'confirm'">
        <div class="flex items-center gap-3">
          <div class="flex h-12 w-12 items-center justify-center rounded-2xl bg-red-500/15 text-2xl">🗑️</div>
          <h2 class="text-xl font-bold">确认卸载极维工具箱</h2>
        </div>
        <p class="mt-4 text-[13px] leading-6 text-[var(--text-sub)]">
          卸载将移除程序本体文件与注册表项。您可以选择是否保留缓存、软件清单与主题配置等用户数据。
          <span v-if="dataDir" class="mt-2 block">用户数据目录：{{ dataDir }}</span>
        </p>
        <label class="mt-5 flex cursor-pointer items-center gap-2.5 text-sm">
          <input v-model="keepData" type="checkbox" class="h-4 w-4 accent-[var(--primary)]" />
          保留本地缓存、软件清单与主题配置
        </label>
        <p v-if="!keepData" class="mt-2 text-xs text-red-500">
          ⚠ 取消勾选将永久删除全部用户数据（缓存、清单、配置与备份），此操作不可恢复。
        </p>
        <div class="mt-7 flex justify-end gap-3">
          <Button variant="gray" @click="closeWin">取消</Button>
          <Button variant="danger" @click="start">确认卸载</Button>
        </div>
      </template>

      <!-- 进度 -->
      <template v-else-if="phase === 'running'">
        <h2 class="text-xl font-bold">正在卸载极维工具箱</h2>
        <div class="mt-6 h-2.5 overflow-hidden rounded-full bg-[var(--border)]">
          <div class="indeterminate h-full w-1/3 rounded-full" style="background: var(--primary)" />
        </div>
        <p class="mt-4 text-[13px] text-[var(--text-sub)]">{{ stageText }}</p>
      </template>

      <!-- 完成 -->
      <template v-else-if="phase === 'done'">
        <div class="flex flex-col items-center py-4">
          <div class="pop-in flex h-16 w-16 items-center justify-center rounded-full bg-emerald-500 text-3xl text-white">✓</div>
          <h2 class="mt-4 text-xl font-bold text-emerald-500">卸载完成</h2>
          <p class="mt-2 text-center text-[13px] text-[var(--text-sub)]">
            {{ keepData ? "用户数据已保留，可随时重新安装。" : "已清除全部程序文件与用户数据。" }}
          </p>
          <p v-if="keepData && dataDir" class="mt-2 break-all text-center text-[11px] text-[var(--text-sub)]">
            数据目录仍在：{{ dataDir }}<br />
            如需彻底清除，可在文件资源管理器中手动删除该目录
          </p>
          <Button variant="primary" class="mt-6" @click="closeWin">关闭窗口</Button>
        </div>
      </template>

      <!-- 失败 -->
      <template v-else>
        <div class="flex flex-col items-center py-4">
          <div class="flex h-16 w-16 items-center justify-center rounded-full bg-red-500 text-3xl text-white">✕</div>
          <h2 class="mt-4 text-xl font-bold text-red-500">卸载失败</h2>
          <p class="mt-2 text-center text-[13px] text-[var(--text-sub)]">{{ errText }}</p>
          <div class="mt-6 flex gap-3">
            <Button variant="gray" @click="retry">返回重试</Button>
            <Button variant="primary" @click="closeWin">关闭窗口</Button>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.indeterminate {
  animation: indet 1.4s ease-in-out infinite;
}
@keyframes indet {
  0% { transform: translateX(-110%); }
  55% { transform: translateX(160%); }
  100% { transform: translateX(-110%); }
}
@media (prefers-reduced-motion: reduce) {
  .indeterminate { animation: none; width: 100%; }
}
</style>
