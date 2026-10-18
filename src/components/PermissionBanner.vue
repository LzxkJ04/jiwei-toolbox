<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Loader2, ShieldAlert, ShieldCheck, X } from "lucide-vue-next";
import Button from "./ui/Button.vue";
import { getEnvStatus, relaunchAsAdmin } from "../lib/tauri";
import { toast } from "../lib/toast";

/**
 * 全局权限提示条。
 * 普通权限：常驻柔和提示，列出受限功能，并提供「以管理员身份重启」。
 * 管理员：显示一次风险提醒横幅（本次会话可关闭）。
 * 关闭状态只存在内存里，重启软件会重新检测——与文档「关闭后本次会话不再弹出」一致。
 */

const isAdmin = ref<boolean | null>(null);
const dismissed = ref(false);
const elevating = ref(false);

/** 普通权限下受限的功能（与后端 is_admin 判定一致的操作） */
const LIMITED = [
  "深度残留卸载",
  "修改系统服务",
  "修改启动项",
  "硬盘底层健康扫描",
  "修改 Hosts",
  "跨目录备份还原",
];

onMounted(async () => {
  const st = await getEnvStatus();
  isAdmin.value = st?.is_admin ?? null;
});

async function elevate() {
  if (elevating.value) return;
  elevating.value = true;
  try {
    await relaunchAsAdmin();
  } catch (err) {
    const msg = String(err);
    toast(msg.includes("取消") ? "已取消管理员授权" : `提升权限失败：${msg}`, "error");
    elevating.value = false;
  }
  // 成功时进程会自行退出，无需复位状态
}

function dismiss() {
  dismissed.value = true;
}
</script>

<template>
  <div v-if="isAdmin !== null && !dismissed" class="px-4 pt-3">
    <!-- 普通权限：常驻提示 -->
    <div
      v-if="!isAdmin"
      class="flex flex-wrap items-start gap-3 rounded-xl border border-[#f5a623]/40 bg-[#f5a623]/10 px-3.5 py-2.5"
    >
      <ShieldAlert class="mt-0.5 h-4 w-4 shrink-0 text-[#f5a623]" />
      <div class="min-w-0 flex-1">
        <p class="text-xs font-medium text-[#b7791f]">当前以普通权限运行，部分功能受限</p>
        <p class="mt-1 text-[11px] leading-relaxed text-[#b7791f]/85">
          受限功能：{{ LIMITED.join("、") }}。以管理员身份重启后可解锁全部能力。
        </p>
      </div>
      <div class="flex shrink-0 items-center gap-1.5">
        <Button variant="primary" size="sm" :disabled="elevating" @click="elevate">
          <Loader2 v-if="elevating" class="h-3.5 w-3.5 animate-spin" />
          以管理员身份重启
        </Button>
        <Button variant="ghost" size="icon" title="本次会话不再提示" @click="dismiss">
          <X class="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>

    <!-- 管理员：风险提醒 -->
    <div
      v-else
      class="flex flex-wrap items-start gap-3 rounded-xl border border-[#e5484d]/35 bg-[#e5484d]/8 px-3.5 py-2.5"
    >
      <ShieldCheck class="mt-0.5 h-4 w-4 shrink-0 text-[#e5484d]" />
      <div class="min-w-0 flex-1">
        <p class="text-xs font-medium text-[#e5484d]">已以管理员权限运行，全部功能已解锁</p>
        <p class="mt-1 text-[11px] leading-relaxed text-[#e5484d]/80">
          管理员模式下可修改注册表、删除受保护文件与系统服务。此类操作不可逆，请确认每一步的目标再执行。
        </p>
      </div>
      <Button variant="ghost" size="icon" class="shrink-0" title="本次会话不再提示" @click="dismiss">
        <X class="h-3.5 w-3.5" />
      </Button>
    </div>
  </div>
</template>
