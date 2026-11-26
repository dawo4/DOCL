<script setup lang="ts">
import { computed, ref } from "vue";
import { openExternal, type ImportFailure } from "../api";
import { notify, retryFailedFile, store, type DownloadJob } from "../store";

/// 任务卡片上的整合包失败清单：按项目列出，可逐个补下，也可去项目页手动下载。
/// 只负责展示与触发，补下结果由 store 写回任务。
const props = defineProps<{ job: DownloadJob }>();

interface Group {
  key: string;
  name: string;
  projectId: number;
  items: ImportFailure[];
  reason: string;
}

/// 同一个项目失败多个文件时合并成一组：用户认的是 Mod 名，不是文件 ID
const groups = computed<Group[]>(() => {
  const byProject = new Map<number, Group>();
  const result: Group[] = [];
  for (const failure of props.job.failures ?? []) {
    if (failure.project_id <= 0) {
      result.push({
        key: `solo-${result.length}`,
        name: failure.name,
        projectId: 0,
        items: [failure],
        reason: failure.reason,
      });
      continue;
    }
    const existing = byProject.get(failure.project_id);
    if (existing) {
      existing.items.push(failure);
      continue;
    }
    const group: Group = {
      key: `project-${failure.project_id}`,
      name: failure.name,
      projectId: failure.project_id,
      items: [failure],
      reason: failure.reason,
    };
    byProject.set(failure.project_id, group);
    result.push(group);
  }
  return result;
});

const MAX_SHOWN = 6;
const total = computed(() => (props.job.failures ?? []).length);
const shown = computed(() => groups.value.slice(0, MAX_SHOWN));
const hiddenCount = computed(() => Math.max(0, groups.value.length - MAX_SHOWN));

const working = ref(false);
/// 这个任务上已经有补下任务在排队或运行：两个任务同时写同一个 .part 会互相截断
const busy = computed(
  () =>
    working.value ||
    store.jobs.some(
      (job) =>
        job.key &&
          (job.key === `retry:${props.job.id}` ||
            job.key.startsWith(`retry-file:${props.job.id}:`)) &&
          (job.status === "queued" ||
            job.status === "running" ||
            job.status === "paused")
    )
);

async function retry(group: Group) {
  working.value = true;
  try {
    for (const item of group.items) {
      await retryFailedFile(props.job, item);
    }
  } finally {
    working.value = false;
  }
}

function openProject(group: Group) {
  // CurseForge 支持按项目 ID 跳转：/projects/<id> 会重定向到项目主页
  openExternal(`https://www.curseforge.com/projects/${group.projectId}`).catch((err) =>
    notify(`打开项目页失败：${String(err)}`, "err")
  );
}
</script>

<template>
  <div class="fails">
    <p class="fails__head">
      {{ total }} 个文件未完成：可以逐个补下，或到项目页手动下载后拖进实例。
    </p>
    <ul class="fails__list">
      <li v-for="group in shown" :key="group.key" class="fails__row">
        <span class="fails__name" :title="group.name">
          {{ group.name }}
          <em v-if="group.items.length > 1">×{{ group.items.length }}</em>
        </span>
        <span class="fails__reason" :title="group.reason">{{ group.reason }}</span>
        <span class="fails__actions">
          <button
            v-if="group.projectId"
            class="fails__btn"
            :disabled="busy"
            @click="retry(group)"
          >
            重试
          </button>
          <button v-if="group.projectId" class="fails__btn" @click="openProject(group)">
            项目页
          </button>
        </span>
      </li>
    </ul>
    <p v-if="hiddenCount" class="fails__more">
      另有 {{ hiddenCount }} 项未列出，点「补下失败文件」可整批重试。
    </p>
  </div>
</template>

<style scoped>
.fails {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
  padding: 7px 9px;
  border-left: 2px solid var(--amber);
  background: color-mix(in srgb, var(--amber) 7%, var(--panel));
}

.fails__head,
.fails__more {
  margin: 0;
  font-size: var(--text-xs);
  line-height: var(--leading);
  color: var(--ink-soft);
}

.fails__more {
  color: var(--muted);
}

.fails__list {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.fails__row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
  font-size: var(--text-xs);
  line-height: var(--leading);
}

.fails__name {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
  color: var(--ink);
}

.fails__name em {
  margin-left: 3px;
  font-style: normal;
  font-weight: 600;
  color: var(--amber);
}

.fails__reason {
  flex: 1 1 0;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted);
}

.fails__actions {
  display: flex;
  flex: none;
  gap: var(--space-2);
}

.fails__btn {
  height: 22px;
  padding: 0 8px;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  background: var(--panel);
  color: var(--ink-soft);
  font-size: var(--text-xs);
  font-weight: 600;
}

.fails__btn:hover:not(:disabled) {
  color: var(--moss);
  border-color: var(--moss);
}

.fails__btn:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}
</style>
