<script setup lang="ts">
import { computed } from "vue";
import WoodButton from "./WoodButton.vue";
import {
  cancelJob,
  clearFinishedJobs,
  dismissJob,
  pauseAllJobs,
  pauseJob,
  resumeAllJobs,
  resumeJob,
  retryContentDeps,
  retryFailedDownloads,
  retryJob,
  store,
  type DownloadJob,
} from "../store";
import { formatBytes, formatDuration, formatSpeed } from "../utils";

const jobs = computed(() => [...store.jobs].sort((a, b) => a.createdAt - b.createdAt));
const activeJobs = computed(() =>
  jobs.value.filter((job) => !isSettled(job))
);
const settled = computed(() => jobs.value.filter((job) => isSettled(job)));
const paused = computed(() => jobs.value.filter((job) => job.status === "paused"));

const totalSpeed = computed(() => {
  let bytes = 0;
  let elapsed = 1;
  for (const job of jobs.value) {
    const p = job.progress;
    if (job.status !== "running" || !p || p.paused) continue;
    bytes += p.bytes_done;
    elapsed = Math.max(elapsed, p.elapsed_ms);
  }
  return formatSpeed(bytes, elapsed);
});

const statusText: Record<DownloadJob["status"], string> = {
  queued: "排队中",
  running: "下载中",
  paused: "已暂停",
  done: "已完成",
  partial: "部分完成",
  failed: "失败",
  cancelled: "已取消",
};

function isSettled(job: DownloadJob) {
  return (
    job.status === "done" ||
    job.status === "partial" ||
    job.status === "failed" ||
    job.status === "cancelled"
  );
}

function percentOf(job: DownloadJob) {
  if (job.status === "done") return 100;
  const p = job.progress;
  if (!p) return 0;
  // 总进度由后端跨阶段累加；阶段内的字节/文件比例只用于步骤行
  if (p.overall > 0) return Math.min(100, p.overall);
  if (p.bytes_total > 0) return Math.min(100, (p.bytes_done / p.bytes_total) * 100);
  if (p.files_total > 0) return Math.min(100, (p.files_done / p.files_total) * 100);
  return job.status === "partial" ? 100 : 0;
}

function speedOf(job: DownloadJob) {
  if (job.status !== "running" || !job.progress) return "";
  return formatSpeed(job.progress.bytes_done, job.progress.elapsed_ms);
}

function sizeOf(job: DownloadJob) {
  const p = job.progress;
  if (!p) return "";
  if (p.bytes_total > 0) return formatBytes(p.bytes_total);
  return p.bytes_done > 0 ? formatBytes(p.bytes_done) : "";
}
</script>

<template>
  <section class="task-board">
    <header class="task-board__head">
      <strong class="task-board__title">下载任务</strong>
      <span class="task-board__speed mono">{{ totalSpeed }}</span>
    </header>

    <div class="task-board__tools">
      <WoodButton
        v-if="jobs.some((job) => job.status === 'running')"
        variant="ghost"
        size="sm"
        @click="pauseAllJobs()"
      >
        全部暂停
      </WoodButton>
      <WoodButton
        v-else-if="paused.length"
        variant="primary"
        size="sm"
        @click="resumeAllJobs()"
      >
        全部继续
      </WoodButton>
      <WoodButton
        v-if="settled.length"
        variant="ghost"
        size="sm"
        @click="clearFinishedJobs()"
      >
        清空已完成
      </WoodButton>
    </div>

    <div v-if="activeJobs.length" class="task-board__group">
      <div class="task-board__label">
        进行中
        <span class="mono">{{ activeJobs.length }}</span>
      </div>
      <article
        v-for="job in activeJobs"
        :key="job.id"
        class="task-card"
        :class="`task-card--${job.status}`"
      >
        <div class="task-card__top">
          <span class="task-card__title" :title="job.title">{{ job.title }}</span>
          <span class="task-card__state">{{ statusText[job.status] }}</span>
        </div>
        <span class="task-card__bar">
          <i :style="{ width: `${percentOf(job)}%` }" />
        </span>
        <div class="task-card__meta mono">
          <span>{{ Math.round(percentOf(job)) }}%</span>
          <span v-if="speedOf(job)">{{ speedOf(job) }}</span>
          <span v-if="job.progress">{{ formatDuration(job.progress.elapsed_ms) }}</span>
        </div>
        <div class="task-card__actions">
          <button v-if="job.status === 'running'" class="task-card__act" @click="pauseJob(job.id)">
            暂停
          </button>
          <button v-if="job.status === 'paused'" class="task-card__act" @click="resumeJob(job.id)">
            继续
          </button>
          <button class="task-card__x" title="取消任务" @click="cancelJob(job.id)">×</button>
        </div>
        <p v-if="job.error" class="task-card__err">{{ job.error }}</p>
      </article>
    </div>

    <div v-if="settled.length" class="task-board__group">
      <div class="task-board__label">
        最近完成
        <span class="mono">{{ settled.length }}</span>
      </div>
      <article
        v-for="job in settled"
        :key="job.id"
        class="task-card"
        :class="`task-card--${job.status}`"
      >
        <div class="task-card__top">
          <span class="task-card__title" :title="job.title">{{ job.title }}</span>
          <span class="task-card__state">{{ statusText[job.status] }}</span>
        </div>
        <span class="task-card__bar">
          <i :style="{ width: `${percentOf(job)}%` }" />
        </span>
        <div class="task-card__foot">
          <span class="mono">
            {{ Math.round(percentOf(job)) }}%
            <em v-if="sizeOf(job)">{{ sizeOf(job) }}</em>
          </span>
          <div class="task-card__end">
            <button
              v-if="job.status === 'partial' && job.retryKind === 'deps'"
              class="task-card__act"
              @click="retryContentDeps(job)"
            >
              重试补前置
            </button>
            <button
              v-else-if="job.status === 'partial'"
              class="task-card__act"
              @click="retryFailedDownloads(job.id)"
            >
              补下失败文件
            </button>
            <button
              v-else-if="job.status === 'failed' || job.status === 'cancelled'"
              class="task-card__act"
              @click="retryJob(job.id)"
            >
              重试
            </button>
            <button class="task-card__x" title="从列表移除" @click="dismissJob(job.id)">×</button>
          </div>
        </div>
        <p v-if="job.error" class="task-card__err">{{ job.error }}</p>
      </article>
    </div>
  </section>
</template>

<style scoped>
.task-board {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  min-width: 0;
  background: var(--panel);
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 2px;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff),
    0 10px 24px rgba(40, 28, 16, 0.16);
}

.task-board__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.task-board__title {
  font-size: 17px;
  font-weight: 700;
  color: var(--ink);
}

.task-board__speed {
  font-size: 11px;
  color: var(--moss);
}

.task-board__tools {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.task-board__group {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.task-board__label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding-top: 2px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--muted);
}

.task-card {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding: 10px 11px;
  min-width: 0;
  background: var(--panel-2);
  border: 1px solid var(--line);
  border-radius: 2px;
  box-shadow: 0 2px 6px rgba(40, 28, 16, 0.08);
}

.task-card--failed {
  border-color: color-mix(in srgb, var(--danger) 42%, var(--line));
}

.task-card__top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}

.task-card__title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
  font-weight: 700;
  color: var(--ink);
}

.task-card__state {
  flex: none;
  font-size: 10.5px;
  font-weight: 600;
  color: var(--muted);
}

.task-card--done .task-card__state {
  color: var(--moss);
}

.task-card--failed .task-card__state,
.task-card--cancelled .task-card__state {
  color: var(--danger);
}

.task-card--paused .task-card__state,
.task-card--partial .task-card__state {
  color: var(--amber);
}

.task-card__bar {
  display: block;
  height: 4px;
  background: color-mix(in srgb, var(--line-strong) 40%, transparent);
  overflow: hidden;
}

.task-card__bar i {
  display: block;
  height: 100%;
  background: var(--moss);
  transition: width 220ms var(--ease-out-quart);
}

.task-card--done .task-card__bar i {
  background: var(--moss-bright);
}

.task-card--failed .task-card__bar i {
  background: var(--danger);
}

.task-card--paused .task-card__bar i,
.task-card--partial .task-card__bar i {
  background: var(--amber);
}

.task-card__meta,
.task-card__foot {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: 10.5px;
  color: var(--muted);
}

.task-card__foot {
  justify-content: space-between;
}

.task-card__end {
  display: flex;
  align-items: center;
  gap: 6px;
}

.task-card__foot em {
  margin-left: 6px;
  font-style: normal;
  color: var(--muted);
}

.task-card__actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}

.task-card__act {
  height: 22px;
  padding: 0 8px;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  background: var(--panel);
  color: var(--ink-soft);
  font-size: 10.5px;
  font-weight: 600;
}

.task-card__act:hover {
  color: var(--moss);
  border-color: var(--moss);
}

.task-card__x {
  width: 20px;
  height: 20px;
  display: grid;
  place-items: center;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  background: var(--panel);
  color: var(--muted);
  font-size: 13px;
  line-height: 1;
}

.task-card__x:hover {
  color: var(--danger);
  border-color: var(--danger);
}

.task-card__err {
  font-size: 10.5px;
  color: var(--danger);
  word-break: break-all;
}
</style>
