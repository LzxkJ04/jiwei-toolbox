<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import TitleBar from "./components/TitleBar.vue";
import SideNav from "./components/SideNav.vue";
import ToastHost from "./components/ui/ToastHost.vue";
import ConfirmDialog from "./components/ui/ConfirmDialog.vue";
import CommandPalette from "./components/CommandPalette.vue";
import { useThemeStore } from "./stores/theme";
import { useRouteStore } from "./stores/route";
import { useFavoritesStore } from "./stores/favorites";
import { useTasksStore } from "./stores/tasks";
import { useSoftwareStore } from "./stores/software";
import { onNavigate, onTrayHided, onExitConfirmRequest, onTrayNotify, forceExit, getLaunchMode } from "./lib/tauri";
import { toast } from "./lib/toast";

import InstallerView from "./views/InstallerView.vue";
import UninstallerView from "./views/UninstallerView.vue";
import RepairView from "./views/RepairView.vue";
import SoftwareLibrary from "./views/SoftwareLibrary.vue";
import FavoritesView from "./views/FavoritesView.vue";
import LocalSoftware from "./views/LocalSoftware.vue";
import TaskQueue from "./views/TaskQueue.vue";
import BackupView from "./views/BackupView.vue";
import HardwareView from "./views/HardwareView.vue";
import SysMonitorView from "./views/SysMonitorView.vue";
import DiskSpaceView from "./views/DiskSpaceView.vue";
import QuickToolsView from "./views/QuickToolsView.vue";
import SysToolsView from "./views/SysToolsView.vue";
import LicenseView from "./views/LicenseView.vue";
import LogsView from "./views/LogsView.vue";
import SettingsView from "./views/SettingsView.vue";

const theme = useThemeStore();
const route = useRouteStore();

// 引导壳多模式：main 主程序 / installer 安装向导 / uninstaller 卸载器 / repair 修复模式
const launchMode = ref<"main" | "installer" | "uninstaller" | "repair" | "unattended">("main");

const views: Record<string, unknown> = {
  library: SoftwareLibrary,
  favorites: FavoritesView,
  local: LocalSoftware,
  tasks: TaskQueue,
  backup: BackupView,
  hardware: HardwareView,
  monitor: SysMonitorView,
  diskspace: DiskSpaceView,
  quicktools: QuickToolsView,
  systools: SysToolsView,
  license: LicenseView,
  logs: LogsView,
  settings: SettingsView,
};
const activeView = computed(() => views[route.active]);

/* ---- 托盘退出确认：任务运行时先弹窗 ---- */
const showExitConfirm = ref(false);
async function confirmExit() {
  try {
    await forceExit();
  } catch (err) {
    console.error("[tauri] force_exit 调用失败:", err);
  }
}

onMounted(async () => {
  launchMode.value = await getLaunchMode();
  // 安装向导 / 卸载器：独立全屏流程，不初始化主程序逻辑
  if (launchMode.value !== "main") return;

  await theme.init();
  void useFavoritesStore().init();
  void useTasksStore().init();
  void useSoftwareStore().load();

  // 托盘菜单跳转
  void onNavigate(({ page }) => route.go(page as never));
  // 首次最小化到托盘提示
  void onTrayHided(() => toast("已最小化到托盘", "info"));
  // 托盘点击退出但任务仍在执行 → 弹确认窗
  void onExitConfirmRequest(() => {
    showExitConfirm.value = true;
  });
  // 托盘操作结果通知（如清理缓存完成）
  void onTrayNotify((msg) => toast(msg, "success"));
});
</script>

<template>
  <!-- 安装向导：左栏品牌 + 右侧 7 页流程 -->
  <InstallerView v-if="launchMode === 'installer'" />
  <!-- 卸载器 -->
  <UninstallerView v-else-if="launchMode === 'uninstaller'" />
  <!-- 修复模式 -->
  <RepairView v-else-if="launchMode === 'repair'" />

  <!-- 主程序 -->
  <div v-else class="flex h-full flex-col overflow-hidden">
    <ToastHost />
    <CommandPalette />
    <TitleBar />
    <div class="flex min-h-0 flex-1">
      <SideNav />
      <main class="app-main">
        <div :key="route.active" class="page-enter page-frame">
          <component :is="activeView" />
        </div>
      </main>
    </div>

    <!-- 托盘退出确认 -->
    <ConfirmDialog
      :open="showExitConfirm"
      title="退出极维工具箱"
      message="任务正在执行，退出会终止所有任务。确定要退出吗？"
      confirm-text="退出并终止任务"
      danger
      @update:open="showExitConfirm = $event"
      @confirm="confirmExit"
    />
  </div>
</template>
