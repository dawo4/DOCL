<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import AppIcon from "./AppIcon.vue";
import IconButton from "./IconButton.vue";
import WoodButton from "./WoodButton.vue";
import {
  cancelJob,
  clearFinishedJobs,
  dismissJob,
  pauseAllJobs,
  pauseJob,
  resumeAllJobs,
  resumeJob,
  retryFailedDownloads,
  retryJob,
  store,
  type DownloadJob,
} from "../store";
import { formatBytes, formatDuration, formatSpeed } from "../utils";

const hovered = ref(false);
let hoverTimer: number | undefined;
let hideTimer: number | undefined;

const jobs = computed(() => [...store.jobs].sort((a, b) => a.createdAt - b.createdAt));
const running = computed(() =>
  jobs.value.filter((job) => job.status === "running" || job.status === "paused")
);
const queued = computed(() => jobs.value.filter((job) => job.status === "queued"));
const settled = computed(() =>
  jobs.value.filter(
    (job) =>
      job.status === "done" ||
      job.status === "partial" ||
      job.status === "failed" ||
      job.status === "cancelled"
  )
);

const active = computed(() => running.value.length > 0);
/// 悬浮球 / 折叠提示展示的主任务
const primary = computed<DownloadJob | null>(
  () => running.value[0] ?? queued.value[0] ?? jobs.value[jobs.value.length - 1] ?? null
);
const primarySnapshot = computed(() => primary.value?.progress ?? null);

const statusText: Record<DownloadJob["status"], string> = {
  queued: "排队中",
  running: "下载中",
  paused: "已暂停",
  done: "已完成",
  partial: "部分完成",
  failed: "失败",
  cancelled: "已取消",
};

const stageText: Record<string, string> = {
  install: "安装游戏文件",
  repair: "检查与修复",
  java: "安装 Java 运行时",
  shader: "安装光影加载器",
  terracotta: "准备联机组件",
};

function percentOf(job: DownloadJob) {
  if (job.status === "done") return 100;
  const p = job.progress;
  if (!p) return 0;
  if (p.bytes_total > 0) return Math.min(100, (p.bytes_done / p.bytes_total) * 100);
  if (p.files_total > 0) return Math.min(100, (p.files_done / p.files_total) * 100);
  return job.status === "partial" ? 100 : 0;
}

/// 总进度按字节加权（没有字节数时退回平均），和 PCL 的「总进度」口径一致
const overallPercent = computed(() => {
  const pool = running.value.length ? running.value : jobs.value;
  if (!pool.length) return 0;
  let done = 0;
  let total = 0;
  for (const job of pool) {
    const p = job.progress;
    if (p && p.bytes_total > 0) {
      done += p.bytes_done;
      total += p.bytes_total;
    }
  }
  if (total > 0) return Math.min(100, (done / total) * 100);
  const sum = pool.reduce((acc, job) => acc + percentOf(job), 0);
  return sum / pool.length;
});

function speedOf(job: DownloadJob) {
  if (job.status !== "running" || !job.progress) return "";
  return formatSpeed(job.progress.bytes_done, job.progress.elapsed_ms);
}

const totalSpeed = computed(() => {
  let bytes = 0;
  let elapsed = 1;
  for (const job of running.value) {
    const p = job.progress;
    if (!p || p.paused) continue;
    bytes += p.bytes_done;
    elapsed = Math.max(elapsed, p.elapsed_ms);
  }
  return formatSpeed(bytes, elapsed);
});

/// 剩余文件：只统计还在跑的任务，排队任务尚未开始计数
const remainingFiles = computed(() =>
  running.value.reduce((acc, job) => {
    const p = job.progress;
    if (!p || !p.files_total) return acc;
    return acc + Math.max(0, p.files_total - p.files_done);
  }, 0)
);

const remainingBytes = computed(() =>
  running.value.reduce((acc, job) => {
    const p = job.progress;
    if (!p || !p.bytes_total) return acc;
    return acc + Math.max(0, p.bytes_total - p.bytes_done);
  }, 0)
);

const ballPercent = computed(() => (primary.value ? percentOf(primary.value) : 0));
const fillStyle = computed(() => ({ "--fill": `${ballPercent.value}%` }));

function stepsOf(job: DownloadJob) {
  const list = job.progress?.steps ?? [];
  if (list.length) return list;
  const fallback = job.progress?.message || stageText[job.progress?.stage ?? ""] || "";
  return fallback ? [{ label: fallback, done: false, percent: percentOf(job) }] : [];
}

function queueIndex(job: DownloadJob) {
  return queued.value.findIndex((item) => item.id === job.id) + 1;
}

function isSettled(job: DownloadJob) {
  return (
    job.status === "done" ||
    job.status === "partial" ||
    job.status === "failed" ||
    job.status === "cancelled"
  );
}

function closePanel() {
  store.downloadPanel = jobs.value.length ? "ball" : "hidden";
}

function openPanel() {
  store.downloadPanel = "open";
  hovered.value = false;
}

function hideBall() {
  if (!active.value) store.downloadPanel = "hidden";
}

function showTip() {
  if (hoverTimer) window.clearTimeout(hoverTimer);
  hoverTimer = window.setTimeout(() => {
    hovered.value = true;
  }, 300);
}

function hideTip() {
  if (hoverTimer) window.clearTimeout(hoverTimer);
  hovered.value = false;
}

watch(
  () => active.value,
  (value, old) => {
    if (hideTimer) window.clearTimeout(hideTimer);
    if (old && !value && store.downloadPanel === "ball") {
      hideTimer = window.setTimeout(() => {
        if (!active.value && store.downloadPanel === "ball") store.downloadPanel = "hidden";
      }, 6000);
    }
  }
);

onBeforeUnmount(() => {
  if (hoverTimer) window.clearTimeout(hoverTimer);
  if (hideTimer) window.clearTimeout(hideTimer);
});
</script>

<template>
  <teleport to="body">
    <transition name="dl-fade">
      <div
        v-if="store.downloadPanel === 'open' && jobs.length"
        class="dl-overlay"
        @mousedown.self="closePanel"
      >
        <section class="dl-panel">
          <header class="dl-panel__head">
            <div class="dl-panel__heading">
              <span class="dl-panel__mark">
                <AppIcon name="download" :size="15" />
              </span>
              <div>
                <h3 class="dl-panel__title">下载任务</h3>
              </div>
            </div>
            <div class="dl-panel__head-actions">
              <WoodButton
                v-if="running.some((job) => job.status === 'running')"
                variant="ghost"
                size="sm"
                @click="pauseAllJobs()"
              >
                全部暂停
              </WoodButton>
              <WoodButton
                v-if="running.some((job) => job.status === 'paused')"
                variant="ghost"
                size="sm"
                @click="resumeAllJobs()"
              >
                全部继续
              </WoodButton>
              <IconButton icon="minimize" label="收起到右下角" :size="28" @click="closePanel" />
            </div>
          </header>

          <div class="dl-board">
            <aside class="dl-kpi">
              <div class="dl-kpi__item">
                <span class="dl-kpi__label">总进度</span>
                <strong class="dl-kpi__value">
                  {{ overallPercent.toFixed(1) }}<em>%</em>
                </strong>
                <span class="dl-kpi__bar"><i :style="{ width: `${overallPercent}%` }" /></span>
              </div>
              <div class="dl-kpi__item">
                <span class="dl-kpi__label">下载速度</span>
                <strong class="dl-kpi__value dl-kpi__value--mono">{{ totalSpeed }}</strong>
              </div>
              <div class="dl-kpi__item">
                <span class="dl-kpi__label">剩余文件</span>
                <strong class="dl-kpi__value dl-kpi__value--mono">{{ remainingFiles }}</strong>
              </div>
              <div class="dl-kpi__item">
                <span class="dl-kpi__label">剩余大小</span>
                <strong class="dl-kpi__value dl-kpi__value--mono">
                  {{ remainingBytes > 0 ? formatBytes(remainingBytes) : "—" }}
                </strong>
              </div>
              <div v-if="queued.length" class="dl-kpi__queue">
                <span class="dl-kpi__label">排队</span>
                <ul>
                  <li v-for="job in queued.slice(0, 4)" :key="job.id" :title="job.title">
                    {{ job.title }}
                  </li>
                </ul>
                <span v-if="queued.length > 4" class="dl-kpi__more">
                  还有 {{ queued.length - 4 }} 个
                </span>
              </div>
            </aside>

            <div class="dl-list">
              <article
                v-for="job in jobs"
                :key="job.id"
                class="dl-card"
                :class="`dl-card--${job.status}`"
              >
                <header class="dl-card__head">
                  <span class="dl-card__state">{{ statusText[job.status] }}</span>
                  <span class="dl-card__title" :title="job.title">{{ job.title }}</span>
                  <span class="dl-card__pct mono">{{ Math.round(percentOf(job)) }}%</span>
                  <button
                    class="dl-card__x"
                    :title="isSettled(job) ? '从列表移除' : '取消任务'"
                    @click="isSettled(job) ? dismissJob(job.id) : cancelJob(job.id)"
                  >
                    ×
                  </button>
                </header>

                <span class="dl-card__bar">
                  <i :style="{ width: `${percentOf(job)}%` }" />
                </span>

                <p v-if="job.status === 'queued'" class="dl-card__note">
                  第 {{ queueIndex(job) }} 位，等待前面的任务结束后自动开始
                </p>

                <ul v-else class="dl-steps">
                  <li
                    v-for="(step, index) in stepsOf(job)"
                    :key="`${index}-${step.label}`"
                    class="dl-steps__row"
                    :class="{ 'dl-steps__row--done': step.done }"
                  >
                    <span class="dl-steps__mark">
                      <AppIcon v-if="step.done" name="check" :size="10" />
                      <span v-else class="dl-steps__pct mono">
                        {{ Math.round(step.percent ?? 0) }}%
                      </span>
                    </span>
                    <span class="dl-steps__label">{{ step.label }}</span>
                  </li>
                </ul>

                <div v-if="!isSettled(job) && job.status !== 'queued'" class="dl-card__meta mono">
                  <span v-if="job.progress && job.progress.bytes_total > 0">
                    {{ formatBytes(job.progress.bytes_done) }} /
                    {{ formatBytes(job.progress.bytes_total) }}
                  </span>
                  <span v-else-if="job.progress">{{ formatBytes(job.progress.bytes_done) }}</span>
                  <span v-if="speedOf(job)">{{ speedOf(job) }}</span>
                  <span v-if="job.progress">{{ formatDuration(job.progress.elapsed_ms) }}</span>
                </div>

                <p v-if="job.error" class="dl-card__error">{{ job.error }}</p>

                <footer class="dl-card__actions">
                  <WoodButton
                    v-if="job.status === 'running'"
                    variant="ghost"
                    size="sm"
                    @click="pauseJob(job.id)"
                  >
                    暂停
                  </WoodButton>
                  <WoodButton
                    v-if="job.status === 'paused'"
                    variant="primary"
                    size="sm"
                    @click="resumeJob(job.id)"
                  >
                    继续
                  </WoodButton>
                  <WoodButton
                    v-if="job.status === 'partial'"
                    variant="primary"
                    size="sm"
                    @click="retryFailedDownloads(job.id)"
                  >
                    补下失败文件
                  </WoodButton>
                  <WoodButton
                    v-if="job.status === 'failed' || job.status === 'cancelled'"
                    variant="primary"
                    size="sm"
                    @click="retryJob(job.id)"
                  >
                    重试
                  </WoodButton>
                </footer>
              </article>
            </div>
          </div>

          <footer class="dl-panel__foot">
            <span class="dl-foot__hint mono">总速度 {{ totalSpeed }}</span>
            <WoodButton
              v-if="settled.length"
              variant="ghost"
              size="sm"
              @click="clearFinishedJobs()"
            >
              清空已完成
            </WoodButton>
            <WoodButton variant="ghost" size="sm" @click="closePanel">收起到悬浮球</WoodButton>
          </footer>
        </section>
      </div>
    </transition>

    <div v-if="store.downloadPanel === 'ball' && jobs.length" class="dl-ball-wrap">
      <transition name="dl-tip">
        <div v-if="hovered" class="dl-tip">
          <div class="dl-tip__head">
            <strong>{{ primary?.title ?? "下载任务" }}</strong>
            <span class="dl-tip__count mono">{{ running.length + queued.length }} 个任务</span>
          </div>
          <div class="dl-tip__bar">
            <span :style="{ width: `${ballPercent}%` }" />
          </div>
          <div class="dl-tip__rows">
            <span>{{ Math.round(ballPercent) }}%</span>
            <span>{{ totalSpeed }}</span>
            <span v-if="(primarySnapshot?.bytes_total ?? 0) > 0">
              {{ formatBytes(primarySnapshot?.bytes_done ?? 0) }} /
              {{ formatBytes(primarySnapshot?.bytes_total ?? 0) }}
            </span>
            <span v-else-if="primarySnapshot">{{ formatBytes(primarySnapshot.bytes_done) }}</span>
          </div>
          <p v-if="primary?.error" class="dl-tip__err">{{ primary.error }}</p>
          <button class="dl-tip__open" @click="openPanel">打开下载详情</button>
        </div>
      </transition>

      <button
        class="dl-ball"
        :class="{
          'dl-ball--done': !active && primary?.status === 'done',
          'dl-ball--failed': primary?.status === 'failed',
          'dl-ball--partial': primary?.status === 'partial',
          'dl-ball--busy': active,
        }"
        :style="fillStyle"
        aria-label="下载进度"
        @mouseenter="showTip"
        @mouseleave="hideTip"
        @click="openPanel"
      >
        <span class="dl-ball__glow" />
        <span class="dl-ball__face">
          <span class="dl-ball__water" />
          <span class="dl-ball__sheen" />
          <span class="dl-ball__content">
            <span class="dl-ball__icon">
              <AppIcon
                :name="
                  primary?.status === 'failed' || primary?.status === 'partial'
                    ? 'alert'
                    : active
                      ? 'download'
                      : 'check'
                "
                :size="13"
              />
            </span>
            <span class="dl-ball__pct">
              {{
                primary?.status === "failed"
                  ? "失败"
                  : primary?.status === "partial"
                    ? "部分"
                    : active
                      ? `${Math.round(ballPercent)}%`
                      : "完成"
              }}
            </span>
          </span>
        </span>
      </button>

      <button v-if="!active" class="dl-ball__dismiss" title="关闭" @click="hideBall">
        ×
      </button>
    </div>
  </teleport>
</template>

<style scoped>
.dl-overlay {
  position: fixed;
  inset: 0;
  z-index: 95;
  display: grid;
  place-items: center;
  background: rgba(40, 28, 16, 0.34);
  backdrop-filter: blur(4px);
  animation: dl-fade-in 180ms ease both;
}

.dl-panel {
  width: min(880px, 94vw);
  max-height: 84vh;
  background: var(--panel);
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 2px;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff),
    0 30px 70px rgba(40, 28, 16, 0.32);
  padding: 18px 20px 14px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  position: relative;
  overflow: hidden;
  animation: dl-pop 280ms var(--ease-out-expo) both;
}

.dl-panel::before {
  content: "";
  position: absolute;
  inset: 0 0 auto;
  height: 2px;
  background: linear-gradient(90deg, var(--moss), var(--moss-bright));
}

.dl-panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
}

.dl-panel__heading {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

/* 标题前的方格图标，和木纹面板的实体按键同一套材质语言 */
.dl-panel__mark {
  width: 30px;
  height: 30px;
  display: grid;
  place-items: center;
  flex-shrink: 0;
  color: var(--moss);
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--panel) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--panel) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--panel) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--panel) 74%, #000);
  background: var(--panel-2);
  box-shadow: inset 0 1px 2px rgba(255, 255, 255, 0.4);
}

.dl-panel__title {
  font-size: 19px;
  font-weight: 800;
  line-height: 1.2;
  color: var(--walnut-deep);
}

.dl-panel__head-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* 左指标 + 右任务列表，结构参考 PCL 的下载任务页 */
.dl-board {
  display: grid;
  grid-template-columns: 168px minmax(0, 1fr);
  gap: 16px;
  min-height: 0;
  flex: 1;
}

.dl-kpi {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 16px 14px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-right-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  background: var(--panel-2);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.1);
}

.dl-kpi__item {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--line);
}

.dl-kpi__item:last-child {
  padding-bottom: 0;
  border-bottom: none;
}

.dl-kpi__label {
  font-size: 10.5px;
  letter-spacing: 0.12em;
  color: var(--muted);
}

.dl-kpi__value {
  font-family: var(--font-display);
  font-size: 26px;
  line-height: 1;
  color: var(--walnut-deep);
  font-variant-numeric: tabular-nums;
}

.dl-kpi__value em {
  margin-left: 2px;
  font-size: 12px;
  font-style: normal;
  color: var(--muted);
}

.dl-kpi__value--mono {
  font-family: var(--font-mono);
  font-size: 15px;
}

.dl-kpi__bar {
  display: block;
  height: 3px;
  margin-top: 2px;
  background: color-mix(in srgb, var(--line-strong) 40%, transparent);
  overflow: hidden;
}

.dl-kpi__bar i {
  display: block;
  height: 100%;
  background: linear-gradient(90deg, var(--moss), var(--moss-bright));
  transition: width 320ms var(--ease-out-quart);
}

/* 排队任务用左栏剩余空间列出来，避免整列留白 */
.dl-kpi__queue {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.dl-kpi__queue ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dl-kpi__queue li {
  font-size: 11px;
  color: var(--ink-soft);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dl-kpi__queue li::before {
  content: "· ";
  color: var(--muted);
}

.dl-kpi__more {
  font-size: 10.5px;
  color: var(--muted);
}

/* 任务列表 */
.dl-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
  overflow-y: auto;
  padding-right: 4px;
}

.dl-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px 10px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--line-strong) 42%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 42%, var(--ink));
  border-right-color: color-mix(in srgb, var(--line-strong) 56%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 56%, #fff);
  background: var(--panel-2);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.1);
  position: relative;
}

/* 左侧状态色条：比整块着色克制，也不影响文字对比度 */
.dl-card::before {
  content: "";
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 4px;
  background: var(--moss);
}

.dl-card--queued::before {
  background: color-mix(in srgb, var(--muted) 58%, var(--walnut-deep));
}

.dl-card--paused::before {
  background: var(--amber, #b8862f);
}

.dl-card--failed::before {
  background: var(--danger);
}

.dl-card--cancelled::before {
  background: color-mix(in srgb, var(--muted) 58%, var(--walnut-deep));
}

.dl-card--partial::before {
  background: var(--amber, #b8862f);
}

.dl-card__head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.dl-card__state {
  flex-shrink: 0;
  padding: 1px 6px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
  color: var(--moss);
  background: var(--moss-soft);
  border: 1px solid color-mix(in srgb, var(--moss) 26%, transparent);
}

.dl-card--queued .dl-card__state {
  color: var(--muted);
  background: color-mix(in srgb, var(--line) 60%, transparent);
  border-color: var(--line-strong);
}

.dl-card--paused .dl-card__state,
.dl-card--partial .dl-card__state {
  color: color-mix(in srgb, var(--amber, #b8862f) 78%, var(--walnut-deep));
  background: color-mix(in srgb, var(--amber, #b8862f) 14%, transparent);
  border-color: color-mix(in srgb, var(--amber, #b8862f) 34%, transparent);
}

.dl-card--failed .dl-card__state,
.dl-card--cancelled .dl-card__state {
  color: var(--danger);
  background: var(--danger-soft);
  border-color: color-mix(in srgb, var(--danger) 30%, transparent);
}

.dl-card__title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
}

.dl-card__pct {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--moss);
}

.dl-card--queued .dl-card__pct {
  color: var(--muted);
}

.dl-card--failed .dl-card__pct {
  color: var(--danger);
}

.dl-card--paused .dl-card__pct {
  color: color-mix(in srgb, var(--amber, #b8862f) 72%, var(--walnut-deep));
}

.dl-card__x {
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  display: grid;
  place-items: center;
  color: var(--muted);
  font-size: 15px;
  line-height: 1;
  border: 1px solid transparent;
  background: transparent;
  transition: color var(--dur-1) ease, border-color var(--dur-1) ease;
}

.dl-card__x:hover {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 40%, transparent);
}

.dl-card__bar {
  display: block;
  height: 3px;
  background: color-mix(in srgb, var(--line-strong) 38%, transparent);
  overflow: hidden;
}

.dl-card__bar i {
  display: block;
  height: 100%;
  background: linear-gradient(90deg, var(--moss), var(--moss-bright));
  transition: width 320ms var(--ease-out-quart);
}

.dl-card--failed .dl-card__bar i {
  background: var(--danger);
}

.dl-card--paused .dl-card__bar i {
  background: var(--amber, #b8862f);
}

.dl-card__note {
  margin: 0;
  font-size: 11px;
  color: var(--muted);
}

/* 步骤明细：完成打勾，进行中显示百分比，和 PCL 的任务卡片一致 */
.dl-steps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
  max-height: 148px;
  overflow-y: auto;
}

.dl-steps__row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--ink-soft);
  min-width: 0;
}

.dl-steps__mark {
  width: 30px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  color: var(--moss);
}

.dl-steps__pct {
  font-size: 10.5px;
  color: var(--moss);
  font-variant-numeric: tabular-nums;
}

.dl-steps__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dl-steps__row--done {
  color: var(--muted);
}

.dl-steps__row--done .dl-steps__label {
  color: var(--muted);
}

.dl-card__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  font-size: 10.5px;
  color: var(--muted);
}

.dl-card__error {
  margin: 0;
  padding: 7px 9px;
  font-size: 11.5px;
  line-height: 1.45;
  color: var(--danger);
  background: var(--danger-soft);
  border: 1px solid color-mix(in srgb, var(--danger) 32%, var(--line-strong));
  overflow-wrap: anywhere;
}

.dl-card__actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.dl-card__actions:empty {
  display: none;
}

.dl-panel__foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 2px;
  border-top: 1px solid var(--line);
}

.dl-foot__hint {
  margin-right: auto;
  padding-top: 8px;
  font-size: 10.5px;
  color: var(--muted);
}

@media (max-width: 760px) {
  .dl-board {
    grid-template-columns: minmax(0, 1fr);
  }

  .dl-kpi {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 12px;
  }

  .dl-kpi__item {
    flex: 1 1 120px;
    padding-bottom: 0;
    border-bottom: none;
  }
}
/* 悬浮球 */
.dl-ball-wrap {
  position: fixed;
  right: 24px;
  bottom: calc(var(--statusbar) + 20px);
  z-index: 92;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 10px;
  animation: dl-ball-float 4.6s ease-in-out infinite;
}

.dl-ball {
  /* 与项目实体按键同一套材质：偏白正面 + 左上亮边 / 右下暗边，底边留厚度 */
  --face: var(--panel);
  --bevel-hi: color-mix(in srgb, var(--face) 52%, #fff);
  --bevel-lo: color-mix(in srgb, var(--face) 74%, #000);
  /* 水位配色，默认苔绿；失败 / 部分完成在下面覆盖 */
  --water-top: color-mix(in srgb, var(--moss) 46%, var(--moss-soft));
  --water-bottom: color-mix(in srgb, var(--moss) 30%, var(--moss-soft));
  --water-line: color-mix(in srgb, var(--moss-bright) 62%, #fff);
  position: relative;
  width: 56px;
  height: 56px;
  padding: 0;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 3px;
  border-top-color: var(--bevel-hi);
  border-left-color: var(--bevel-hi);
  border-right-color: var(--bevel-lo);
  border-bottom-color: var(--bevel-lo);
  background: var(--face);
  box-shadow: 0 12px 26px rgba(40, 28, 16, 0.24), 0 2px 4px rgba(40, 28, 16, 0.12);
  transition: transform var(--dur-2) var(--ease-out-expo), box-shadow var(--dur-2) ease,
    background 150ms ease;
}

.dl-ball:hover {
  transform: translateY(-3px);
  box-shadow: 0 18px 32px rgba(40, 28, 16, 0.28);
}

.dl-ball:active {
  transform: translateY(2px);
  border-top-color: var(--bevel-lo);
  border-left-color: var(--bevel-lo);
  border-right-color: var(--bevel-hi);
  border-bottom-color: var(--bevel-hi);
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.24);
}

/* 外侧柔光：下载中缓慢呼吸，完成/失败时静止 */
.dl-ball__glow {
  position: absolute;
  inset: -7px;
  z-index: -1;
  border-radius: 4px;
  background: radial-gradient(closest-side, rgba(var(--accent-rgb), 0.4), transparent 74%);
  filter: blur(7px);
  opacity: 0.3;
  pointer-events: none;
}

.dl-ball--busy .dl-ball__glow {
  animation: dl-ball-breathe 2.6s ease-in-out infinite;
}

/* 内凹的方槽：进度水从槽底往上灌 */
.dl-ball__face {
  position: absolute;
  inset: 2px;
  overflow: hidden;
  border-radius: 1px;
  background: var(--panel-2);
  box-shadow: inset 0 2px 4px rgba(70, 48, 26, 0.16),
    inset 0 0 0 1px color-mix(in srgb, var(--line-strong) 55%, transparent);
}

.dl-ball__water {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: var(--fill, 0%);
  background: linear-gradient(180deg, var(--water-top), var(--water-bottom));
  transition: height 360ms var(--ease-out-quart), background 240ms ease;
}

/* 水线：一道受光细边，像水面反光 */
.dl-ball__water::before {
  content: "";
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 2px;
  background: var(--water-line);
}

.dl-ball--busy .dl-ball__water::before {
  animation: dl-ball-wave 2.2s ease-in-out infinite;
}

/* 表面扫过的高光，让方块看起来是玻璃材质 */
.dl-ball__sheen {
  position: absolute;
  top: -45%;
  left: -70%;
  width: 48%;
  height: 190%;
  transform: rotate(20deg);
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.42), transparent);
  pointer-events: none;
  animation: dl-ball-sheen 3.4s ease-in-out infinite;
}

/* 图标与百分比浮在水面之上 */
.dl-ball__content {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1px;
  color: var(--walnut-deep);
  text-shadow: 0 1px 0 rgba(255, 255, 255, 0.6);
}

.dl-ball__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  color: inherit;
}

.dl-ball--busy .dl-ball__icon {
  animation: dl-ball-icon 1.8s ease-in-out infinite;
}

.dl-ball__pct {
  font-size: 10.5px;
  font-weight: 700;
  line-height: 1.1;
}

.dl-ball--done {
  animation: dl-ball-pop 520ms var(--ease-out-expo);
}

.dl-ball--done .dl-ball__content {
  color: var(--moss);
}

.dl-ball--failed {
  --water-top: color-mix(in srgb, var(--danger) 40%, var(--danger-soft));
  --water-bottom: color-mix(in srgb, var(--danger) 26%, var(--danger-soft));
  --water-line: color-mix(in srgb, var(--danger) 24%, #fff);
  animation: dl-ball-shake 460ms ease;
}

.dl-ball--failed .dl-ball__content {
  color: var(--danger);
}

.dl-ball--partial {
  --water-top: color-mix(in srgb, var(--amber) 44%, var(--panel-2));
  --water-bottom: color-mix(in srgb, var(--amber) 28%, var(--panel-2));
  --water-line: color-mix(in srgb, var(--amber) 34%, #fff);
}

.dl-ball--partial .dl-ball__content {
  color: color-mix(in srgb, var(--amber) 62%, var(--walnut-deep));
}

.dl-ball__dismiss {
  width: 22px;
  height: 22px;
  border-radius: 2px;
  background: var(--panel);
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--panel) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--panel) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--panel) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--panel) 74%, #000);
  color: var(--muted);
  font-size: 14px;
  line-height: 1;
  display: grid;
  place-items: center;
  margin-right: 4px;
  box-shadow: 0 4px 10px rgba(40, 28, 16, 0.14);
  animation: dl-ball-pop 360ms var(--ease-out-expo);
}

.dl-ball__dismiss:hover {
  color: var(--danger);
  border-color: var(--danger);
}

@keyframes dl-ball-float {
  0%,
  100% {
    transform: translateY(0);
  }
  50% {
    transform: translateY(-4px);
  }
}

@keyframes dl-ball-breathe {
  0%,
  100% {
    opacity: 0.28;
    transform: scale(0.96);
  }
  50% {
    opacity: 0.72;
    transform: scale(1.06);
  }
}

@keyframes dl-ball-sheen {
  0% {
    left: -70%;
  }
  55%,
  100% {
    left: 120%;
  }
}

/* 水线轻微起伏，读作水面在晃 */
@keyframes dl-ball-wave {
  0%,
  100% {
    transform: translateY(0);
    opacity: 0.85;
  }
  50% {
    transform: translateY(-1px);
    opacity: 1;
  }
}

@keyframes dl-ball-icon {
  0%,
  100% {
    transform: translateY(1px);
  }
  50% {
    transform: translateY(-2px);
  }
}

@keyframes dl-ball-pop {
  0% {
    transform: scale(0.86);
  }
  55% {
    transform: scale(1.08);
  }
  100% {
    transform: scale(1);
  }
}

@keyframes dl-ball-shake {
  0%,
  100% {
    transform: translateX(0);
  }
  25% {
    transform: translateX(-4px);
  }
  50% {
    transform: translateX(4px);
  }
  75% {
    transform: translateX(-2px);
  }
}

.dl-tip {
  width: 260px;
  padding: 12px 13px;
  border-radius: 0;
  background: color-mix(in srgb, var(--ink) 94%, transparent);
  color: var(--paper);
  box-shadow: 0 14px 34px rgba(0, 0, 0, 0.24);
  border: 1px solid color-mix(in srgb, var(--paper) 12%, transparent);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dl-tip__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 12.5px;
}

.dl-tip__bar {
  height: 5px;
  border-radius: 0;
  background: color-mix(in srgb, var(--paper) 18%, transparent);
  overflow: hidden;
}

.dl-tip__bar span {
  display: block;
  height: 100%;
  border-radius: 0;
  background: linear-gradient(90deg, var(--moss-bright), var(--amber));
  transition: width 240ms var(--ease-out-quart);
}

.dl-tip__rows {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  font-size: 11px;
  color: color-mix(in srgb, var(--paper) 72%, transparent);
}

.dl-tip__count {
  font-size: 11px;
  color: color-mix(in srgb, var(--paper) 66%, transparent);
}

.dl-tip__err {
  font-size: 11px;
  color: color-mix(in srgb, var(--danger) 45%, var(--paper));
}

.dl-tip__open {
  height: 28px;
  border-radius: 0;
  background: color-mix(in srgb, var(--paper) 12%, transparent);
  color: var(--paper);
  font-size: 11.5px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--paper) 22%, transparent);
  border-left-color: color-mix(in srgb, var(--paper) 22%, transparent);
  border-right-color: color-mix(in srgb, #000 26%, transparent);
  border-bottom-color: color-mix(in srgb, #000 34%, transparent);
  transition: background var(--dur-1) ease, transform var(--dur-1) ease;
}

.dl-tip__open:hover {
  background: color-mix(in srgb, var(--paper) 20%, transparent);
  transform: translateY(-1px);
}

.dl-tip__open:active {
  transform: translateY(1px);
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.35);
}

@keyframes dl-fade-in {
  from {
    opacity: 0;
  }
}

@keyframes dl-pop {
  from {
    opacity: 0;
    transform: translateY(16px) scale(0.97);
  }
}

.dl-fade-enter-active,
.dl-fade-leave-active {
  transition: opacity 160ms ease;
}

.dl-fade-enter-from,
.dl-fade-leave-to {
  opacity: 0;
}

.dl-tip-enter-active,
.dl-tip-leave-active {
  transition: opacity 160ms ease, transform 160ms var(--ease-out-expo);
}

.dl-tip-enter-from,
.dl-tip-leave-to {
  opacity: 0;
  transform: translateY(6px);
}

</style>
