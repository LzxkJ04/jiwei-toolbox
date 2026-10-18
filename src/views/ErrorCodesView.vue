<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ChevronDown, ChevronRight, Copy, RefreshCw, Search } from "lucide-vue-next";
import { invoke } from "@tauri-apps/api/core";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Input from "../components/ui/Input.vue";
import Badge from "../components/ui/Badge.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import { copyText } from "../lib/utils";
import { toast } from "../lib/toast";

/* ------------------------------------------------------------------ 类型 */

type Level = "INFO" | "WARN" | "ERROR" | "FATAL";

interface ErrorCode {
  code: string;
  level: Level;
  desc: string;
  hint: string;
}

/* ------------------------------------------------------------------ 模块说明 */

const MODULES: { prefix: string; label: string }[] = [
  { prefix: "MOD01", label: "软件库&下载" },
  { prefix: "MOD02", label: "卸载" },
  { prefix: "MOD03", label: "备份还原" },
  { prefix: "MOD04", label: "硬件检测" },
  { prefix: "MOD05", label: "系统工具" },
  { prefix: "MOD06", label: "配置" },
  { prefix: "MOD07", label: "更新" },
  { prefix: "MOD08", label: "通用底层" },
];

/* ------------------------------------------------------------------ 排查步骤（前端维护，后端无此字段） */

const FIX_STEPS: Record<string, string[]> = {
  "MOD01-001": [
    "检查系统代理/VPN 是否开启：开了代理但代理客户端没启动时，所有下载都会失败",
    "到「设置 → 网络」点一次「测试代理」，确认代理端口能连通",
    "换一个软件源（设置里切换镜像）再试，排除单个源不可用",
    "关闭 Windows 防火墙对工具箱的出站拦截后重试",
  ],
  "MOD01-002": [
    "删除已下载的安装包缓存，让程序重新下载完整文件",
    "确认软件源提供的哈希与包体一致，切换镜像后重新下载",
    "如果走的是公司网络，检查是否被中间设备替换了安装包（HTTPS 劫持）",
  ],
  "MOD01-003": [
    "确认下载目录所在磁盘的剩余空间，至少留出安装包体积的 2 倍",
    "把下载目录改到空间更大的盘（设置 → 目录设置）",
    "清理临时文件：Windows 设置 → 系统 → 存储 → 临时文件",
  ],
  "MOD02-001": [
    "在任务管理器里结束占用该文件的进程，或直接重启电脑后重试卸载",
    "确认要卸载的程序本身已退出（含托盘图标、后台服务）",
    "关闭杀毒软件的实时扫描后重试，部分安全软件会锁住 exe/dll",
    "被系统进程占用时（如 explorer.exe），选择「稍后重启删除」",
  ],
  "MOD02-002": [
    "关闭工具箱，右键「以管理员身份运行」后重新卸载",
    "确认当前账户不是受限账户，且未被组策略禁止写 HKLM",
    "若该项被 TrustedInstaller 保护，改用程序自带的卸载器处理",
  ],
  "MOD03-001": [
    "重新生成一次备份包，传输中断或磁盘写入异常都会导致校验失败",
    "确认备份文件未被压缩软件、同步盘（OneDrive）改动过",
    "备份包所在磁盘做一次错误检查（chkdsk），排除坏道导致的文件损坏",
  ],
  "MOD03-002": [
    "确认密码大小写与输入法状态，注意首尾是否有空格",
    "找回密码：备份包不支持重置密码，只能使用创建时设置的密码",
    "换一台机器导入时确认没有额外输入法自动补全字符",
  ],
  "MOD04-001": [
    "更新主板芯片组与传感器驱动，或安装厂商的监控工具后再检测",
    "部分笔记本/虚拟机不暴露传感器，属正常现象，可忽略该项",
    "以管理员身份运行工具箱，部分传感器需要驱动级权限",
  ],
  "MOD04-002": [
    "立即停止压力测试，等待温度降到 70°C 以下再继续",
    "检查散热：风扇转速、出风口积尘、硅脂是否干涸",
    "降低测试强度或缩短时长，笔记本不要放在床上/包里跑满载测试",
  ],
  "MOD05-001": [
    "以管理员身份重新启动工具箱，Hosts 文件位于系统目录，普通权限无法写入",
    "检查 Hosts 文件是否被设为「只读」，右键属性去掉只读勾选",
    "关闭占用 Hosts 的安全软件（部分杀软会锁定该文件）",
  ],
  "MOD06-001": [
    "确认导入的配置文件是工具箱导出的 JSON，且未被手工改坏（缺引号/多逗号）",
    "用文本编辑器打开检查是否有乱码，另存为 UTF-8 后重试",
    "导入失败时先用「重置」恢复默认配置，再逐项手工设置",
  ],
  "MOD06-002": [
    "确认导入策略：选择「覆盖」会用文件里的值替换本机设置",
    "记录冲突项后重新导入，避免把别的机器的路径配置带过来",
    "涉及目录路径的配置项，导入后到「设置 → 目录设置」核对一遍",
  ],
  "MOD07-001": [
    "确认网络可用后点「重试」，更新包支持断点续传时会接着下",
    "关闭代理或切换到稳定网络，更新服务器对代理链路的兼容性较差",
    "磁盘空间不足也会表现为下载失败，先确认临时目录有余量",
  ],
  "MOD07-002": [
    "软件已自动回滚，当前版本可正常使用，不需要重装",
    "关闭杀毒软件后重试更新，文件替换阶段被杀软拦截是最常见原因",
    "确认工具箱安装目录不在受保护位置（如 Program Files 且无写权限），必要时以管理员运行",
    "多次失败可手动下载完整安装包覆盖安装",
  ],
  "MOD08-001": [
    "无需处理：这是异常关闭后的任务状态保存记录，属正常保护机制",
    "下次打开工具箱时在「任务队列」里确认任务是否需要继续",
    "频繁出现说明程序被强制结束，检查是否被杀软或关机流程中断",
  ],
  "MOD08-002": [
    "确认目标目录存在且当前账户有写权限，必要时以管理员身份运行",
    "检查文件是否被其他程序占用（同类 MOD02-001 的处理方式）",
    "确认磁盘未满、未处于只读/写保护状态（U 盘、网络盘常见）",
  ],
};

/* ------------------------------------------------------------------ 数据 */

const rows = ref<ErrorCode[] | null>(null);
const loading = ref(false);
const keyword = ref("");
const prefix = ref("");
const expanded = ref<string | null>(null);

async function load() {
  loading.value = true;
  try {
    const list = await invoke<ErrorCode[]>("error_code_table");
    rows.value = Array.isArray(list) ? list : [];
  } catch (e) {
    rows.value = [];
    toast(`读取错误码表失败：${String(e)}`, "error");
  } finally {
    loading.value = false;
  }
}

const filtered = computed(() => {
  const list = rows.value ?? [];
  const kw = keyword.value.trim().toLowerCase();
  return list.filter((r) => {
    if (prefix.value && !r.code.startsWith(prefix.value)) return false;
    if (!kw) return true;
    const steps = (FIX_STEPS[r.code] ?? []).join(" ");
    return `${r.code} ${r.desc} ${r.hint} ${steps}`.toLowerCase().includes(kw);
  });
});

const levelCount = computed(() => {
  const s = { WARN: 0, ERROR: 0, FATAL: 0 };
  for (const r of rows.value ?? []) {
    if (r.level === "WARN") s.WARN += 1;
    else if (r.level === "ERROR") s.ERROR += 1;
    else if (r.level === "FATAL") s.FATAL += 1;
  }
  return s;
});

function moduleLabel(code: string): string {
  const p = code.split("-")[0];
  return MODULES.find((m) => m.prefix === p)?.label ?? "未分类";
}

function toggle(code: string) {
  expanded.value = expanded.value === code ? null : code;
}

async function copyCode(code: string) {
  const ok = await copyText(code);
  toast(ok ? `已复制 ${code}` : "复制失败", ok ? "success" : "error");
}

async function copyAll() {
  if (filtered.value.length === 0) {
    toast("当前没有可复制的错误码", "info");
    return;
  }
  const text = filtered.value
    .map((r) => `${r.code}\t${r.level}\t${r.desc}\t${r.hint}`)
    .join("\n");
  const ok = await copyText(text);
  toast(
    ok ? `已复制 ${filtered.value.length} 条错误码` : "复制失败",
    ok ? "success" : "error"
  );
}

onMounted(load);
</script>

<template>
  <div>
    <PageHeader title="错误码速查" desc="运行日志里出现的错误码含义、处理提示与推荐排查步骤">
      <template #extra>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="gray" size="sm" :disabled="loading" @click="load">
            <RefreshCw class="h-3.5 w-3.5" /> 刷新
          </Button>
          <Button variant="gray" size="sm" @click="copyAll">
            <Copy class="h-3.5 w-3.5" /> 复制全部
          </Button>
        </div>
      </template>
    </PageHeader>

    <!-- 错误码规则 -->
    <Card class="mb-4 p-5">
      <h4 class="mb-1 font-semibold text-[var(--text-strong)]">编码规则</h4>
      <p class="text-xs leading-relaxed text-[var(--muted)]">
        错误码格式为「模块-编号」，例如 <span class="font-mono text-[var(--text)]">MOD01-001</span>
        表示软件库&下载模块的第 1 号错误。点下方模块可只看该模块的错误码。
      </p>
      <div class="mt-3 flex flex-wrap gap-1.5">
        <button
          type="button"
          class="rounded-full px-2.5 py-1 text-xs font-medium transition-colors"
          :class="
            prefix === ''
              ? 'bg-[var(--primary)] text-white'
              : 'bg-[var(--btn-gray)] text-[var(--muted)] hover:text-[var(--text)]'
          "
          @click="prefix = ''"
        >
          全部模块
        </button>
        <button
          v-for="m in MODULES"
          :key="m.prefix"
          type="button"
          class="rounded-full px-2.5 py-1 text-xs font-medium transition-colors"
          :class="
            prefix === m.prefix
              ? 'bg-[var(--primary)] text-white'
              : 'bg-[var(--btn-gray)] text-[var(--muted)] hover:text-[var(--text)]'
          "
          @click="prefix = m.prefix"
        >
          <span class="font-mono">{{ m.prefix }}</span>
          <span class="ml-1">{{ m.label }}</span>
        </button>
      </div>
      <div class="mt-3 flex flex-wrap items-center gap-2 text-[11px] text-[var(--muted)]">
        <span>共 {{ rows?.length ?? 0 }} 个错误码</span>
        <Badge tone="warning">WARN {{ levelCount.WARN }}</Badge>
        <Badge tone="danger">ERROR {{ levelCount.ERROR }}</Badge>
        <Badge tone="danger" class="font-bold">FATAL {{ levelCount.FATAL }}</Badge>
      </div>
    </Card>

    <!-- 搜索 -->
    <div class="mb-4 flex flex-wrap items-center gap-3">
      <div class="flex min-w-[220px] flex-1 items-center gap-2">
        <Search class="h-4 w-4 shrink-0 text-[var(--muted)]" />
        <Input v-model="keyword" placeholder="搜索错误码、描述、提示或排查步骤…" class="flex-1" />
      </div>
      <span class="text-xs text-[var(--muted)]">显示 {{ filtered.length }} / {{ rows?.length ?? 0 }} 条</span>
    </div>

    <!-- 表格 -->
    <Card class="p-0">
      <Spinner v-if="loading && rows === null" label="正在读取错误码表…" />
      <EmptyState
        v-else-if="(rows?.length ?? 0) === 0"
        icon="🧾"
        text="没有读到错误码表"
        sub="请点右上角刷新重试。"
      />
      <EmptyState
        v-else-if="filtered.length === 0"
        icon="🔍"
        text="没有匹配的错误码"
        sub="换个关键字，或点「全部模块」清除模块筛选。"
      />
      <div v-else class="overflow-x-auto">
        <table class="w-full min-w-[880px] text-left text-xs">
          <thead>
            <tr class="border-b border-[var(--card-border)] bg-[var(--card-soft)] text-[var(--muted)]">
              <th class="w-[42px] px-2 py-2.5" />
              <th class="w-[112px] px-3 py-2.5 font-medium">错误码</th>
              <th class="w-[80px] px-3 py-2.5 font-medium">等级</th>
              <th class="w-[170px] px-3 py-2.5 font-medium">描述</th>
              <th class="px-3 py-2.5 font-medium">提示文案</th>
              <th class="w-[74px] px-3 py-2.5 text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-[var(--card-border)]">
            <template v-for="r in filtered" :key="r.code">
              <tr
                class="cursor-pointer align-top transition-colors hover:bg-[var(--card-soft)]"
                :class="
                  r.level === 'FATAL'
                    ? 'border-l-2 border-l-[#b91c1c]'
                    : r.level === 'ERROR'
                      ? 'border-l-2 border-l-[#e5484d]'
                      : 'border-l-2 border-l-[#f59e0b]'
                "
                @click="toggle(r.code)"
              >
                <td class="px-2 py-2 text-[var(--muted)]">
                  <component
                    :is="expanded === r.code ? ChevronDown : ChevronRight"
                    class="h-3.5 w-3.5"
                  />
                </td>
                <td class="whitespace-nowrap px-3 py-2">
                  <span class="font-mono text-[var(--text-strong)]">{{ r.code }}</span>
                  <p class="mt-0.5 text-[10px] text-[var(--muted)]">{{ moduleLabel(r.code) }}</p>
                </td>
                <td class="px-3 py-2">
                  <Badge
                    :tone="r.level === 'WARN' ? 'warning' : 'danger'"
                    :class="r.level === 'FATAL' ? 'font-bold' : ''"
                  >
                    {{ r.level }}
                  </Badge>
                </td>
                <td class="break-all px-3 py-2 font-medium text-[var(--text)]">{{ r.desc }}</td>
                <td class="break-all px-3 py-2 leading-relaxed text-[var(--muted)]">{{ r.hint }}</td>
                <td class="px-3 py-2 text-right">
                  <Button
                    variant="ghost"
                    size="sm"
                    class="text-[var(--muted)] hover:text-[var(--primary)]"
                    :title="`复制 ${r.code}`"
                    @click.stop="copyCode(r.code)"
                  >
                    <Copy class="h-3 w-3" />
                  </Button>
                </td>
              </tr>

              <!-- 展开：建议排查步骤 -->
              <tr v-if="expanded === r.code" class="bg-[var(--card-soft)]">
                <td class="px-2 py-3 text-[var(--muted)]" />
                <td colspan="5" class="px-3 py-3">
                  <p class="mb-2 flex flex-wrap items-center gap-2 text-[11px] font-medium text-[var(--text-strong)]">
                    建议排查步骤
                    <span class="font-mono font-normal text-[var(--muted)]">{{ r.code }} · {{ moduleLabel(r.code) }}</span>
                  </p>
                  <ol v-if="(FIX_STEPS[r.code] ?? []).length" class="space-y-1.5">
                    <li
                      v-for="(step, si) in FIX_STEPS[r.code]"
                      :key="si"
                      class="flex min-w-0 gap-2 text-xs leading-relaxed text-[var(--text)]"
                    >
                      <span
                        class="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-[var(--btn-gray)] text-[10px] font-semibold text-[var(--muted)]"
                      >
                        {{ si + 1 }}
                      </span>
                      <span class="min-w-0 break-all">{{ step }}</span>
                    </li>
                  </ol>
                  <p v-else class="text-xs text-[var(--muted)]">暂无针对该错误码的排查建议。</p>
                  <div class="mt-3 flex flex-wrap items-center gap-2">
                    <Button variant="ghost" size="sm" @click.stop="copyCode(r.code)">
                      <Copy class="h-3.5 w-3.5" /> 复制错误码
                    </Button>
                    <span class="text-[10px] text-[var(--muted)]">在日志页搜这个码，能看到触发时的完整上下文。</span>
                  </div>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>
    </Card>

    <p class="mt-3 px-1 text-[11px] leading-relaxed text-[var(--muted)]">
      WARN 表示可继续但有风险，ERROR 表示当前操作失败，FATAL 表示功能已自动中止（如更新回滚、超温停机），需要优先处理。
    </p>
  </div>
</template>
