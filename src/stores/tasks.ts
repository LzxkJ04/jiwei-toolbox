import { defineStore } from "pinia";
import {
  getSetting, setSetting, onInstallProgress, onQueueState, onInstallSummary,
  pauseTasks, resumeTasks, terminateTasks, setTaskCompleteAction, queueInstall,
  type QueueStateInfo, type InstallProgress, type InstallSummary,
} from "../lib/tauri";
import { toast } from "../lib/toast";
import type { SoftwareItem } from "../lib/tauri";

export type TaskStatus = "pending" | "running" | "paused" | "done" | "failed";

/** 任务类型：安装 / 卸载 / 备份导出 / 自定义 */
export type TaskKind = "install" | "uninstall" | "backup" | "custom";

export interface QueueTask {
  id: string;
  name: string;
  kind: TaskKind;
  status: TaskStatus;
  progress: number; // 0-100
  message: string;
  /** 下载速度（由 install-progress 累积估算） */
  speed?: string;
  /** 剩余时间（由速度与总量估算） */
  eta?: string;
  /** 返回码 / 失败原因摘要 */
  code?: string;
  // 内部累积字段（不参与 UI 渲染）
  _lastMb?: number;
  _lastTs?: number;
}

export type TaskFilter = "all" | "install" | "uninstall" | "backup";

export type FinishAction = "none" | "notify" | "log" | "shutdown" | "sleep";

export const FINISH_ACTIONS: { id: FinishAction; label: string }[] = [
  { id: "none", label: "无操作" },
  { id: "notify", label: "弹出通知" },
  { id: "log", label: "打开日志中心" },
  { id: "shutdown", label: "自动关机" },
  { id: "sleep", label: "自动休眠" },
];

/** 前端动作 id → 后端 set_task_complete_action 取值 */
const FINISH_ACTION_BACKEND: Record<FinishAction, string> = {
  none: "none",
  notify: "notify",
  log: "log",
  shutdown: "shutdown",
  sleep: "hibernate",
};

const LS_ACTION = "jw-task-finish-action";
let timer: number | null = null;
let listenersBound = false;

/**
 * 任务队列：安装任务由后端串行队列驱动（install-progress / queue-state 事件），
 * 卸载 / 备份等无后端引擎的类型本地模拟推进。
 */
export const useTasksStore = defineStore("tasks", {
  state: () => ({
    tasks: [] as QueueTask[],
    finishAction: "none" as FinishAction,
    running: false,
    paused: false,
    /** 后端安装队列是否在运行（queue-state 推送） */
    backendRunning: false,
    backendPaused: false,
    filter: "all" as TaskFilter,
    loaded: false,
  }),
  getters: {
    filteredTasks(state): QueueTask[] {
      if (state.filter === "all") return state.tasks;
      return state.tasks.filter((t) =>
        state.filter === "backup" ? t.kind === "backup" : t.kind === state.filter
      );
    },
    totalProgress(state): number {
      if (state.tasks.length === 0) return 0;
      const sum = state.tasks.reduce((acc, t) => acc + Math.min(100, Math.max(0, t.progress)), 0);
      return Math.round(sum / state.tasks.length);
    },
    doneCount: (s) => s.tasks.filter((t) => t.status === "done").length,
    failedCount: (s) => s.tasks.filter((t) => t.status === "failed").length,
    pendingCount: (s) => s.tasks.filter((t) => t.status === "pending" || t.status === "paused").length,
    hasActive: (s) => s.tasks.some((t) => t.status === "running"),
    /** 全局状态文字 */
    statusText(state): string {
      if (state.paused) return "已暂停";
      if (state.running) return "任务执行中";
      if (state.tasks.length === 0) return "等待中";
      const done = state.tasks.filter((t) => t.status === "done").length;
      const failed = state.tasks.filter((t) => t.status === "failed").length;
      if (done + failed >= state.tasks.length) {
        return failed > 0 ? "出错" : "全部完成";
      }
      return "等待中";
    },
  },
  actions: {
    async init() {
      if (this.loaded) return;
      this.loaded = true;
      const a = await getSetting("task_finish_action");
      if (a === "none" || a === "notify" || a === "log" || a === "shutdown" || a === "sleep") {
        this.finishAction = a;
      } else {
        try {
          const la = localStorage.getItem(LS_ACTION);
          if (la === "none" || la === "notify" || la === "log" || la === "shutdown" || la === "sleep") {
            this.finishAction = la;
          }
        } catch {
          /* ignore */
        }
      }
      this.bindListeners();
    },
    /** 注册后端事件监听（仅一次） */
    bindListeners() {
      if (listenersBound) return;
      listenersBound = true;
      void onInstallProgress((p) => this.applyProgress(p));
      void onQueueState((s) => this.applyQueueState(s));
      void onInstallSummary((s) => this.applySummary(s));
    },
    async setFinishAction(a: FinishAction) {
      this.finishAction = a;
      try {
        localStorage.setItem(LS_ACTION, a);
      } catch {
        /* ignore */
      }
      await setSetting("task_finish_action", a);
      // 同步持久化到后端 settings.json（sleep → hibernate）
      await setTaskCompleteAction(FINISH_ACTION_BACKEND[a]);
    },
    /** 从软件库勾选加入队列 */
    addFromSoftware(items: SoftwareItem[]) {
      let added = 0;
      for (const s of items) {
        if (this.tasks.some((t) => t.id === s.id)) continue;
        this.tasks.push({
          id: s.id,
          name: s.name,
          kind: "install",
          status: "pending",
          progress: 0,
          message: "等待中",
        });
        added++;
      }
      return added;
    },
    /** 加入卸载 / 备份等任务（无后端引擎的类型本地模拟推进） */
    addTask(name: string, kind: TaskKind) {
      const id = `${kind}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      this.tasks.push({
        id,
        name,
        kind,
        status: "pending",
        progress: 0,
        message: "等待中",
      });
      return id;
    },
    /** 拖拽排序：把 fromId 移动到 toId 的位置 */
    move(fromId: string, toId: string) {
      const from = this.tasks.findIndex((t) => t.id === fromId);
      const to = this.tasks.findIndex((t) => t.id === toId);
      if (from < 0 || to < 0 || from === to) return;
      const [item] = this.tasks.splice(from, 1);
      this.tasks.splice(to, 0, item);
    },
    remove(id: string) {
      const idx = this.tasks.findIndex((t) => t.id === id);
      if (idx >= 0) this.tasks.splice(idx, 1);
    },
    clear() {
      this.stop();
      this.tasks = [];
    },
    /** 开始 / 继续（安装任务走后端队列，其余本地模拟） */
    async start() {
      if (this.tasks.length === 0) return;
      if (this.paused) {
        await this.resume();
        return;
      }
      if (this.running) return;
      const pending = this.tasks.filter((t) => t.status === "pending");
      if (pending.length === 0) return;
      this.running = true;
      this.paused = false;
      const installIds = pending.filter((t) => t.kind === "install").map((t) => t.id);
      if (installIds.length > 0) {
        const first = this.tasks.find((t) => t.id === installIds[0]);
        if (first) {
          first.status = "running";
          first.message = "开始下载";
        }
        await queueInstall(installIds);
      }
      if (timer === null) {
        timer = window.setInterval(() => this.tick(), 200);
      }
    },
    /** 暂停：后端原子暂停 + 本地模拟暂停 */
    async pause() {
      this.paused = true;
      for (const t of this.tasks) {
        if (t.status === "running") {
          t.status = "paused";
          t.message = "已暂停";
        }
      }
      await pauseTasks();
    },
    /** 继续 */
    async resume() {
      this.paused = false;
      const first = this.tasks.find((t) => t.status === "paused");
      if (first) {
        first.status = "running";
        first.message = "执行中";
      }
      await resumeTasks();
      if (this.running && timer === null) {
        timer = window.setInterval(() => this.tick(), 200);
      }
    },
    /** 终止全部任务（deleteCache 决定是否删除下载缓存） */
    async terminate(deleteCache: boolean) {
      await terminateTasks(deleteCache);
      this.stop();
    },
    stop() {
      this.running = false;
      this.paused = false;
      for (const t of this.tasks) {
        if (t.status === "running" || t.status === "paused" || t.status === "pending") {
          t.status = "failed";
          t.progress = 0;
          t.message = "已终止";
        }
      }
      this.clearTimer();
    },
    clearTimer() {
      if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    },
    /** 本地模拟推进：仅处理无后端引擎的类型（卸载 / 备份 / 自定义） */
    tick() {
      if (this.paused) return;
      const sim = this.tasks.find((t) => t.kind !== "install" && t.status === "running");
      if (sim) {
        sim.progress = Math.min(100, sim.progress + Math.random() * 6 + 2);
        sim.message = sim.progress >= 100 ? "完成" : `正在处理 ${Math.round(sim.progress)}%`;
        if (sim.progress >= 100) sim.status = "done";
        return;
      }
      const nextSim = this.tasks.find((t) => t.kind !== "install" && t.status === "pending");
      if (nextSim) {
        nextSim.status = "running";
        nextSim.message = "执行中";
        return;
      }
      // 后端安装队列仍在跑：等待其事件驱动
      if (this.backendRunning) return;
      const busy = this.tasks.some((t) => t.status === "running" || t.status === "pending");
      if (!busy) {
        this.running = false;
        this.clearTimer();
        this.onAllDone();
      }
    },
    /** install-progress 事件：更新进度 / 速度 / 剩余时间 / 返回码 */
    applyProgress(p: InstallProgress) {
      const t = this.tasks.find((x) => x.id === p.id);
      if (!t) return;
      if (t.status === "pending" || t.status === "paused") t.status = "running";
      // 后端按阶段推送 pct（各阶段从 0 重新计），换算为整任务进度
      if (p.stage === "download" || p.stage === "retry") {
        t.progress = Math.min(90, Math.max(0, p.pct) * 0.9);
      } else if (p.stage === "verify") {
        t.progress = 90 + Math.max(0, p.pct) * 0.04;
      } else if (p.stage === "install") {
        t.progress = 94;
      } else if (p.stage === "done") {
        t.progress = 100;
        t.status = "done";
        t.code = "0";
      } else if (p.stage === "error") {
        t.status = "failed";
        t.code = p.msg;
      } else if (p.stage === "skip") {
        t.status = "failed";
        t.message = "已终止";
      }
      if (p.msg) t.message = p.msg;
      // 速度 / 剩余时间：解析 "已下载 X MB / 总计 Y MB"
      const m = /已下载 (\d+(?:\.\d+)?) MB \/ 总计 (\d+(?:\.\d+)?) MB/.exec(p.msg);
      if (m) {
        const mb = Number(m[1]);
        const totalMb = Number(m[2]);
        const now = Date.now();
        if (t._lastMb !== undefined && t._lastTs !== undefined && now > t._lastTs) {
          const mbps = (mb - t._lastMb) / ((now - t._lastTs) / 1000);
          if (mbps > 0.01) {
            t.speed = `${mbps >= 1 ? mbps.toFixed(1) : mbps.toFixed(2)} MB/s`;
            const remainMb = Math.max(0, totalMb - mb);
            const etaSec = Math.round(remainMb / mbps);
            t.eta = etaSec >= 60 ? `约 ${Math.floor(etaSec / 60)} 分 ${etaSec % 60} 秒` : `约 ${etaSec} 秒`;
          }
        }
        t._lastMb = mb;
        t._lastTs = now;
      }
    },
    /** queue-state 事件：后端队列运行状态 */
    applyQueueState(s: QueueStateInfo) {
      this.backendRunning = s.running;
      this.backendPaused = s.paused;
      if (s.running && !s.paused) {
        this.running = true;
      }
      if (s.running && s.currentId) {
        const cur = this.tasks.find((t) => t.id === s.currentId);
        if (cur && (cur.status === "pending" || cur.status === "paused")) {
          cur.status = "running";
        }
      }
      if (!s.running) {
        // 后端队列结束：若无其他任务在跑则收尾
        const busy = this.tasks.some((t) => t.status === "running" || t.status === "pending");
        if (!busy && this.running) {
          this.running = false;
          this.clearTimer();
        }
      }
    },
    /** install-summary 事件：按后端汇总结果落状态 */
    applySummary(s: InstallSummary) {
      for (const id of s.success) {
        const t = this.tasks.find((x) => x.id === id);
        if (t) {
          t.status = "done";
          t.progress = 100;
          t.message = "安装成功";
          t.code = "0";
        }
      }
      for (const f of s.failed) {
        const t = this.tasks.find((x) => x.id === f.id);
        if (t) {
          t.status = "failed";
          t.message = f.reason;
          t.code = f.reason;
        }
      }
      const total = s.success.length + s.failed.length;
      if (total > 0) {
        toast(`安装队列结束：成功 ${s.success.length} 个${s.failed.length > 0 ? `，失败 ${s.failed.length} 个` : ""}`, s.failed.length > 0 ? "error" : "success");
      }
    },
    onAllDone() {
      if (this.finishAction === "notify") {
        toast("任务队列已全部完成", "success");
      } else if (this.finishAction === "log") {
        toast("任务已全部完成，可在日志中心查看记录", "info");
      }
      // shutdown / sleep 由后端 run_complete_action 执行（60 秒缓冲，可 shutdown /a 取消）
    },
  },
});
