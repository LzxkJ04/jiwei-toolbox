<script setup lang="ts">
import PageHeader from "../components/ui/PageHeader.vue";
import Card from "../components/ui/Card.vue";
import EmptyState from "../components/ui/EmptyState.vue";
</script>

<template>
  <div>
    <PageHeader title="日志中心" desc="查看程序运行日志" />
    <Card><EmptyState icon="📄" text="页面迁移中" sub="该功能将在下一轮迭代中上线" /></Card>
  </div>
</template>
