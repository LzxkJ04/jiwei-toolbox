<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RefreshCw, ShieldAlert, ShieldCheck } from "lucide-vue-next";
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Spinner from "../components/ui/Spinner.vue";
import EmptyState from "../components/ui/EmptyState.vue";
import { licenseStatus, type LicenseStatus } from "../lib/tauri";

const data = ref<LicenseStatus | null>(null);
const loading = ref(false);
const failed = ref(false);

async function load() {
  loading.value = true;
  failed.value = false;
  const res = await licenseStatus();
  loading.value = false;
  if (!res) {
    failed.value = true;
    data.value = null;
    return;
  }
  data.value = res;
}

onMounted(() => {
  void load();
});

function dash(s?: string) {
  const t = (s ?? "").trim();
  return t || "—";
}

const partialShown = computed(() => {
  const key = (data.value?.partialKey ?? "").trim();
  return key ? `*****-${key}` : "—";
});
</script>

<template>
  <div>
    <PageHeader title="许可证状态" desc="只读查看 Windows 授权状态，不会修改或激活系统">
      <template #extra>
        <Button variant="gray" size="sm" :disabled="loading" @click="load">
          <RefreshCw class="h-3.5 w-3.5" :class="loading && 'animate-spin'" />
          重新查询
        </Button>
      </template>
    </PageHeader>

    <Spinner v-if="loading && !data" label="正在查询许可证状态…" />

    <Card v-else-if="failed">
      <EmptyState icon="🪪" text="查询失败" sub="无法读取许可证信息，请稍后重试" />
    </Card>

    <div v-else-if="data" class="grid grid-cols-1 gap-5 lg:grid-cols-2">
      <Card class="p-5 lg:col-span-2">
        <div class="flex items-start gap-4">
          <div
            class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl"
            :class="data.licensed ? 'bg-[#30a46c]/15 text-[#30a46c]' : 'bg-[#f5a623]/15 text-[#f5a623]'"
          >
            <ShieldCheck v-if="data.licensed" class="h-6 w-6" />
            <ShieldAlert v-else class="h-6 w-6" />
          </div>
          <div class="min-w-0">
            <p class="text-lg font-bold text-[var(--text-strong)]">{{ dash(data.statusText) }}</p>
            <p class="mt-1 text-sm text-[var(--muted)]">
              {{
                data.licensed
                  ? "当前 Windows 处于已授权状态。"
                  : "当前不是「已授权」。本页只展示查询结果，不提供激活操作。"
              }}
            </p>
          </div>
        </div>
      </Card>

      <Card class="p-5">
        <p class="mb-1 text-xs text-[var(--muted)]">系统版本</p>
        <p class="text-sm font-semibold text-[var(--text-strong)]">{{ dash(data.osEdition) }}</p>
        <p class="mt-1 text-xs text-[var(--muted)]">{{ dash(data.osVersion) }}</p>
      </Card>

      <Card class="p-5">
        <p class="mb-1 text-xs text-[var(--muted)]">部分产品密钥</p>
        <p class="font-mono text-sm font-semibold text-[var(--text-strong)]">{{ partialShown }}</p>
        <p class="mt-1 text-xs text-[var(--muted)]">仅显示后五位，不会读取完整密钥</p>
      </Card>

      <Card class="p-5 lg:col-span-2">
        <p class="mb-1 text-xs text-[var(--muted)]">产品 ID</p>
        <p class="break-all font-mono text-sm font-semibold text-[var(--text-strong)]">{{ dash(data.productId) }}</p>
      </Card>
    </div>
  </div>
</template>
