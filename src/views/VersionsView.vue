﻿﻿﻿<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import defaultProfileCard from "../assets/profile-card.jpg";
import defaultProfileHero from "../assets/profile-hero.jpg";
import { api, openFolder, type PlayStats } from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import InstanceGlyph from "../components/InstanceGlyph.vue";
import SkinHead from "../components/SkinHead.vue";
import WoodButton from "../components/WoodButton.vue";
import {
  launchVersion,
  loadInstanceIconImage,
  openInstance,
  refreshDirCounts,
  sameDir,
  selectVersion,
  selectedAccount,
  selectedVersion,
  setInstanceIcon,
  setInstanceIconImage,
  store,
  switchGameDir,
  uninstallVersion,
} from "../store";
import { ICON_PRESETS, formatBytes, formatDate, kindLabel } from "../utils";

const confirmId = ref<string | null>(null);
let confirmTimer: number | undefined;

/// 正在展开图标选择器的版本
const pickerFor = ref<string | null>(null);
/// 目录切换面板是否展开
const dirOpen = ref(false);
/// 自定义图标的 data URL 缓存，避免每次渲染都读盘
const customIcons = reactive<Record<string, string>>({});

/// 和 PCL 一致：整合包继承的原版 / 加载器版本不单独占卡片
const visibleVersions = computed(() =>
  store.installed.filter((item) => !item.dependency)
);

/// 卡片上方那颗头颅的皮肤与种子，跟启动台用同一套
const skinSource = computed(
  () => store.accountSkin?.data_url || store.defaultSkin
);
const skinSeed = computed(() => selectedAccount.value?.uuid ?? "steve");

function requestDelete(id: string) {
  if (confirmId.value === id) {
    confirmId.value = null;
    uninstallVersion(id);
    return;
  }
  confirmId.value = id;
  if (confirmTimer) window.clearTimeout(confirmTimer);
  confirmTimer = window.setTimeout(() => {
    if (confirmId.value === id) confirmId.value = null;
  }, 3200);
}

function openDir() {
  if (store.config?.game_dir) openFolder(store.config.game_dir);
}

function displayName(id: string) {
  const name = store.metas[id]?.display_name;
  return name && name.trim() ? name : id;
}

/// 整合包实例的 id 是包名，这里补上真实的游戏版本与加载器
function versionTag(item: { id: string; game_version?: string; loader?: string }) {
  const parts: string[] = [];
  if (item.game_version && item.game_version !== item.id) parts.push(item.game_version);
  if (item.loader && item.loader !== "vanilla") {
    parts.push(item.loader.charAt(0).toUpperCase() + item.loader.slice(1));
  }
  return parts.join(" · ");
}

const CATEGORY_LABELS: Record<string, string> = {
  survival: "生存",
  creative: "创造",
  pvp: "PvP",
  modpack: "整合包",
  tech: "生电",
  build: "建筑",
  other: "其他",
};

function categoryLabel(value?: string) {
  if (!value) return "";
  return CATEGORY_LABELS[value] ?? value;
}

function iconOf(id: string) {
  return store.metas[id]?.icon || "auto";
}

/* ---------- 游戏目录切换 ---------- */

const gameDirs = computed(() => store.config?.game_dirs ?? []);
const activeDir = computed(() => store.config?.game_dir ?? "");

async function toggleDirs() {
  if (dirOpen.value) {
    dirOpen.value = false;
    return;
  }
  pickerFor.value = null;
  dirOpen.value = true;
  await refreshDirCounts();
}

async function pickDir(path: string) {
  if (sameDir(path, activeDir.value)) {
    dirOpen.value = false;
    return;
  }
  pickerFor.value = null;
  confirmId.value = null;
  await switchGameDir(path);
  dirOpen.value = false;
  await syncIcons();
}

function manageDirs() {
  dirOpen.value = false;
  store.page = "settings";
}

function dirCount(path: string) {
  return store.dirCounts[path] ?? 0;
}

/// 元数据变化后补齐自定义图标缓存
async function syncIcons() {
  for (const item of visibleVersions.value) {
    if (iconOf(item.id) !== "custom") {
      delete customIcons[item.id];
      continue;
    }
    if (customIcons[item.id]) continue;
    const data = await loadInstanceIconImage(item.id);
    if (data) customIcons[item.id] = data;
  }
}

onMounted(syncIcons);
watch(() => store.metas, syncIcons, { deep: true });

/* ---------- 本地游玩画像 ---------- */

const stats = ref<PlayStats | null>(null);

async function loadStats() {
  try {
    stats.value = await api.playStats();
  } catch {
    stats.value = null;
  }
}

onMounted(loadStats);
// 游戏退出后刷新一次，时长立即可见
watch(
  () => store.gameRunning,
  (running) => {
    if (!running) void loadStats();
  }
);

/* ---------- 卡片底图（可在外观页替换） ---------- */

const heroBg = ref(defaultProfileHero);
const cardBg = ref(defaultProfileCard);
let bgSeq = 0;

async function loadCardBackgrounds() {
  const seq = (bgSeq += 1);
  let hero = defaultProfileHero;
  let card = defaultProfileCard;
  const config = store.config;
  if (config?.profile_hero_image) {
    try {
      hero = await api.backgroundDataUrl(config.profile_hero_image);
    } catch {
      // 文件不存在时保持内置图
    }
  }
  if (config?.profile_card_image) {
    try {
      card = await api.backgroundDataUrl(config.profile_card_image);
    } catch {
      // 同上
    }
  }
  if (seq !== bgSeq) return;
  heroBg.value = hero;
  cardBg.value = card;
}

onMounted(loadCardBackgrounds);
watch(
  () => [store.config?.profile_hero_image, store.config?.profile_card_image],
  () => void loadCardBackgrounds()
);

const versionStats = computed(() => stats.value?.versions ?? {});

/// 卡片文字颜色：设置里指定时覆盖默认浅色，并按亮度自动配对比阴影
const profileStyle = computed<Record<string, string>>(() => {
  const color = store.config?.profile_text_color?.trim();
  if (!color) return {} as Record<string, string>;
  const hex = color.replace("#", "");
  let luminance = 0.5;
  if (hex.length === 6) {
    const r = parseInt(hex.slice(0, 2), 16) / 255;
    const g = parseInt(hex.slice(2, 4), 16) / 255;
    const b = parseInt(hex.slice(4, 6), 16) / 255;
    luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }
  return {
    "--profile-ink": color,
    "--profile-soft": `color-mix(in srgb, ${color} 78%, transparent)`,
    "--profile-muted": `color-mix(in srgb, ${color} 56%, transparent)`,
    "--profile-shadow":
      luminance > 0.6
        ? "0 1px 4px rgba(18, 12, 6, 0.62)"
        : "0 1px 4px rgba(255, 252, 244, 0.6)",
  };
});

const totalSeconds = computed(() =>
  Object.values(versionStats.value).reduce((sum, item) => sum + item.total_seconds, 0)
);

const totalLaunches = computed(() =>
  Object.values(versionStats.value).reduce((sum, item) => sum + item.launches, 0)
);

const topVersion = computed(() => {
  const entries = Object.entries(versionStats.value).filter(
    ([, stat]) => stat.total_seconds > 0
  );
  if (!entries.length) return null;
  const [id, stat] = entries.reduce((best, current) =>
    current[1].total_seconds > best[1].total_seconds ? current : best
  );
  return { id, stat };
});

const lastSession = computed(() => stats.value?.sessions[0] ?? null);

const maxPlay = computed(() =>
  Math.max(
    1,
    ...Object.values(versionStats.value).map((item) => item.total_seconds)
  )
);

const favCount = computed(
  () => visibleVersions.value.filter((item) => store.metas[item.id]?.favorite).length
);

function playOf(id: string) {
  return versionStats.value[id] ?? null;
}

function playSeconds(id: string) {
  return versionStats.value[id]?.total_seconds ?? 0;
}

/// 时长展示：大数字用紧凑写法，行内用「X 小时 Y 分」
function formatPlaytime(seconds: number, compact = false) {
  if (!seconds) return compact ? "—" : "0 分钟";
  if (seconds < 60) return "不足 1 分钟";
  // 先四舍五入到总分钟再拆分，避免出现「X 小时 60 分」
  const totalMinutes = Math.round(seconds / 60);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (compact && hours >= 1) return `${hours} 小时`;
  if (hours > 0) return minutes ? `${hours} 小时 ${minutes} 分` : `${hours} 小时`;
  return `${minutes} 分钟`;
}

/// 该版本占总游玩时长的比例，用于行内细进度条
function playShare(id: string) {
  const stat = playOf(id);
  if (!stat || !stat.total_seconds) return 0;
  return Math.max(3, Math.round((stat.total_seconds / maxPlay.value) * 100));
}

function lastPlayLabel(id: string) {
  const stat = playOf(id);
  if (!stat?.last_played) return "";
  return formatDate(new Date(stat.last_played * 1000).toISOString());
}

/// 该版本占全部游玩时长的比例（用于时长分布与文案）
function playTotalShare(id: string) {
  if (!totalSeconds.value) return 0;
  return Math.round((playSeconds(id) / totalSeconds.value) * 100);
}

/// 总时长拆成「小时 + 分钟」，统计区用大字号分两段展示
const heroMinutesTotal = computed(() =>
  totalSeconds.value ? Math.max(1, Math.round(totalSeconds.value / 60)) : 0
);
const heroHours = computed(() => Math.floor(heroMinutesTotal.value / 60));
const heroMinutes = computed(() => heroMinutesTotal.value % 60);

/// 所有版本里最早的一次游玩，用于「入坑于」文案
const firstPlayed = computed(() => {
  let earliest = 0;
  for (const stat of Object.values(versionStats.value)) {
    if (stat.first_played && (!earliest || stat.first_played < earliest)) {
      earliest = stat.first_played;
    }
  }
  return earliest;
});

interface WeekDay {
  key: string;
  label: string;
  seconds: number;
  today: boolean;
}

function dayKey(date: Date) {
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

/// 最近 7 天（含今天）的游玩时长，用于统计区的小柱图
const weekDays = computed<WeekDay[]>(() => {
  const now = new Date();
  const days: WeekDay[] = [];
  for (let offset = 6; offset >= 0; offset -= 1) {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - offset);
    days.push({
      key: dayKey(date),
      label: "日一二三四五六".charAt(date.getDay()),
      seconds: 0,
      today: offset === 0,
    });
  }
  const bucket = new Map(days.map((day) => [day.key, day]));
  for (const session of stats.value?.sessions ?? []) {
    const hit = bucket.get(dayKey(new Date(session.started * 1000)));
    if (hit) hit.seconds += session.seconds;
  }
  return days;
});

const weekMax = computed(() =>
  Math.max(1, ...weekDays.value.map((day) => day.seconds))
);

function weekBar(seconds: number) {
  if (!seconds) return "2px";
  return `${Math.max(12, Math.round((seconds / weekMax.value) * 100))}%`;
}

function relativeDay(timestamp: number) {
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const played = new Date(timestamp * 1000);
  const start = new Date(
    played.getFullYear(),
    played.getMonth(),
    played.getDate()
  ).getTime();
  const days = Math.round((today - start) / 86_400_000);
  if (days <= 0) return "今天";
  if (days === 1) return "昨天";
  return `${days} 天前`;
}

/// 占比配色全部取自主题变量，换主题时自动跟着变
const DIST_COLORS = [
  "var(--moss)",
  "color-mix(in srgb, var(--moss) 62%, var(--walnut-light))",
  "var(--walnut-light)",
  "color-mix(in srgb, var(--moss) 40%, var(--walnut))",
  "var(--muted)",
];

interface DistSlice {
  id: string;
  name: string;
  seconds: number;
  percent: number;
  color: string;
}

/// 各版本时长占比：取前四，其余合并成「其他版本」
const distribution = computed<DistSlice[]>(() => {
  const total = totalSeconds.value;
  if (!total) return [];
  const entries = Object.entries(versionStats.value)
    .filter(([, stat]) => stat.total_seconds > 0)
    .sort((a, b) => b[1].total_seconds - a[1].total_seconds);
  if (!entries.length) return [];
  const slices: DistSlice[] = entries.slice(0, 4).map(([id, stat], index) => ({
    id,
    name: displayName(id),
    seconds: stat.total_seconds,
    percent: (stat.total_seconds / total) * 100,
    color: DIST_COLORS[index],
  }));
  const rest = entries
    .slice(4)
    .reduce((sum, [, stat]) => sum + stat.total_seconds, 0);
  if (rest > 0) {
    slices.push({
      id: "__rest",
      name: "其他版本",
      seconds: rest,
      percent: (rest / total) * 100,
      color: DIST_COLORS[4],
    });
  }
  return slices;
});

function togglePicker(id: string) {
  pickerFor.value = pickerFor.value === id ? null : id;
}

async function choosePreset(id: string, preset: string) {
  pickerFor.value = null;
  await setInstanceIcon(id, preset);
}

async function chooseImage(id: string) {
  pickerFor.value = null;
  const source = await openDialog({
    multiple: false,
    title: "选择版本图标",
    filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] }],
  });
  if (!source || typeof source !== "string") return;
  await setInstanceIconImage(id, source);
  const data = await loadInstanceIconImage(id);
  if (data) customIcons[id] = data;
}
</script>

<template>
  <div class="page">
    <header class="page-head page-head--split">
      <div class="page-head__lead">
        <h1 class="page-title">版本库</h1>
      </div>
      <div class="page-head__actions">
        <div class="dir-switch">
          <IconButton icon="layers" label="切换游戏目录" :size="32" @click="toggleDirs" />
          <template v-if="dirOpen">
            <div class="dir-backdrop" @click="dirOpen = false" />
            <div class="dir-panel">
              <span class="dir-panel__title">游戏目录</span>
              <button
                v-for="dir in gameDirs"
                :key="dir.path"
                class="dir-item"
                :class="{ 'dir-item--active': sameDir(dir.path, activeDir) }"
                @click="pickDir(dir.path)"
              >
                <span class="dir-item__body">
                  <strong>{{ dir.name }}</strong>
                  <span class="dir-item__path mono">{{ dir.path }}</span>
                </span>
                <span class="dir-item__count mono">{{ dirCount(dir.path) }}</span>
                <AppIcon
                  v-if="sameDir(dir.path, activeDir)"
                  name="check"
                  :size="14"
                />
              </button>
              <button class="dir-manage" @click="manageDirs">管理目录…</button>
            </div>
          </template>
        </div>
        <IconButton icon="folder" label="打开游戏目录" :size="32" @click="openDir" />
        <WoodButton variant="primary" size="sm" sheen @click="store.page = 'download'">
          <AppIcon name="plus" :size="14" />
          安装新版本
        </WoodButton>
      </div>
    </header>

    <div v-if="visibleVersions.length" class="shelf-stats" :style="profileStyle">
      <article
        class="shelf-hero"
        :class="{ 'shelf-hero--empty': !distribution.length }"
        :style="{ '--hero-bg': `url(${heroBg})` }"
      >
        <div class="hero-total">
          <span class="hero-total__eyebrow">游玩档案</span>
          <div v-if="totalSeconds" class="hero-total__num">
            <template v-if="heroHours">
              <strong>{{ heroHours }}</strong>
              <span class="hero-total__unit">小时</span>
              <strong>{{ heroMinutes }}</strong>
              <span class="hero-total__unit">分</span>
            </template>
            <template v-else>
              <strong>{{ heroMinutes }}</strong>
              <span class="hero-total__unit">分钟</span>
            </template>
          </div>
          <div v-else class="hero-total__num hero-total__num--empty">还没有游玩记录</div>
          <div v-if="totalSeconds" class="hero-facts">
            <span><b>{{ totalLaunches }}</b> 次启动</span>
            <span v-if="firstPlayed">
              <b>{{ formatDate(new Date(firstPlayed * 1000).toISOString()) }}</b> 入坑
            </span>
          </div>
          <div v-if="lastSession" class="hero-recent">
            <span class="hero-recent__label">最近游玩</span>
            <strong v-if="!topVersion || lastSession.version !== topVersion.id">
              {{ displayName(lastSession.version) }}
            </strong>
            <span class="hero-recent__meta">{{ relativeDay(lastSession.started) }}</span>
          </div>
        </div>

        <div class="hero-bottom">
          <div class="week">
            <span class="week__title">最近 7 天</span>
            <div class="week__bars">
              <div v-for="day in weekDays" :key="day.key" class="week__col">
                <div class="week__track">
                  <span
                    class="week__bar"
                    :class="{
                      'week__bar--today': day.today,
                      'week__bar--empty': !day.seconds,
                    }"
                    :style="{ height: weekBar(day.seconds) }"
                    :title="
                      day.seconds ? formatPlaytime(day.seconds) : '没有游玩记录'
                    "
                  />
                </div>
                <span class="week__label" :class="{ 'week__label--today': day.today }">
                  {{ day.today ? "今天" : `周${day.label}` }}
                </span>
              </div>
            </div>
          </div>

          <div v-if="distribution.length" class="hero-dist">
            <div class="hero-dist__bar">
              <span
                v-for="slice in distribution"
                :key="slice.id"
                :style="{ width: `${slice.percent}%`, background: slice.color }"
                :title="`${slice.name} · ${formatPlaytime(slice.seconds)}`"
              />
            </div>
            <div class="hero-dist__legend">
              <span v-for="slice in distribution" :key="slice.id" class="hero-dist__item">
                <i :style="{ background: slice.color }" />
                <b>{{ slice.name }}</b>
                <em>{{ Math.round(slice.percent) }}%</em>
              </span>
            </div>
          </div>
        </div>
      </article>

      <aside class="shelf-side">
        <div class="shelf-head" aria-hidden="true">
          <SkinHead :skin="skinSource" :seed="skinSeed" :height="120" />
        </div>

        <article
          class="stat-card stat-card--top"
          :style="{ '--card-bg': `url(${cardBg})` }"
        >
          <span class="stat-card__label">最常玩版本</span>
          <div v-if="topVersion" class="top-version">
            <span class="top-version__tile">
              <img v-if="customIcons[topVersion.id]" :src="customIcons[topVersion.id]" alt="" />
              <InstanceGlyph v-else :preset="iconOf(topVersion.id)" :size="24" />
            </span>
            <div class="top-version__body">
              <strong>{{ displayName(topVersion.id) }}</strong>
            </div>
          </div>
          <div v-else class="stat-empty">还没有游玩记录</div>

          <div class="top-overview">
            <span><b>{{ visibleVersions.length }}</b> 个版本</span>
            <span><b>{{ favCount }}</b> 个收藏</span>
          </div>
        </article>
      </aside>
    </div>

    <div v-if="!visibleVersions.length" class="blank">
      <p class="blank__title">版本架还是空的</p>
      <p class="blank__hint">从右上角安装新版本，或直接把整合包压缩包拖进窗口</p>
    </div>

    <div v-else class="version-list">
      <article
        v-for="(item, index) in visibleVersions"
        :key="item.id"
        class="version-row reveal"
        :class="{
          'version-row--active': item.id === selectedVersion?.id,
          'version-row--picking': pickerFor === item.id,
        }"
        :style="{ '--i': index }"
      >
        <div class="version-row__icon-wrap">
          <button
            class="version-row__icon"
            :title="`更换 ${displayName(item.id)} 的图标`"
            @click.stop="togglePicker(item.id)"
          >
            <img v-if="customIcons[item.id]" :src="customIcons[item.id]" alt="" />
            <InstanceGlyph v-else :preset="iconOf(item.id)" :size="26" />
          </button>

          <template v-if="pickerFor === item.id">
            <div class="picker-backdrop" @click.stop="pickerFor = null" />
            <div class="icon-picker" @click.stop>
              <span class="icon-picker__title">选择图标</span>
              <div class="icon-picker__grid">
                <button
                  v-for="preset in ICON_PRESETS"
                  :key="preset.value"
                  class="icon-picker__chip"
                  :class="{ 'icon-picker__chip--active': iconOf(item.id) === preset.value }"
                  @click="choosePreset(item.id, preset.value)"
                >
                  <span class="icon-picker__tile" :style="{ '--tile': preset.color }">
                    <InstanceGlyph :preset="preset.value" :size="18" />
                  </span>
                  {{ preset.label }}
                </button>
              </div>
              <button class="icon-picker__upload" @click="chooseImage(item.id)">
                <AppIcon name="image" :size="14" />
                自定义图片…
              </button>
            </div>
          </template>
        </div>

        <div class="version-row__main">
          <div class="version-row__title">
            <strong class="display">{{ displayName(item.id) }}</strong>
            <span v-if="store.metas[item.id]?.favorite" class="tag tag--grass">收藏</span>
            <span v-if="item.id === selectedVersion?.id" class="tag tag--grass">当前</span>
            <span v-if="topVersion?.id === item.id" class="tag">最常玩</span>
            <span class="tag">{{ kindLabel(item.kind) }}</span>
          </div>
          <span class="version-row__sub mono">
            <template v-if="versionTag(item)">{{ versionTag(item) }} · </template>
            {{ item.id }}
            <template v-if="store.metas[item.id]?.category && store.metas[item.id]?.category !== 'auto'">
              · {{ categoryLabel(store.metas[item.id]?.category) }}
            </template>
            <template v-if="playOf(item.id)?.last_played">
              · 上次 {{ lastPlayLabel(item.id) }}
            </template>
          </span>
          <span v-if="playShare(item.id)" class="version-row__share">
            <span class="version-row__share-track">
              <span :style="{ width: `${playShare(item.id)}%` }" />
            </span>
            <span class="version-row__share-num mono">{{ playTotalShare(item.id) }}%</span>
          </span>
        </div>

        <div v-if="playOf(item.id)" class="version-row__play">
          <strong>{{ formatPlaytime(playSeconds(item.id), true) }}</strong>
          <span>{{ playOf(item.id)?.launches }} 次启动</span>
        </div>

        <div class="version-row__meta">
          <span><i>客户端</i>{{ formatBytes(item.jar_size) }}</span>
          <span><i>Java</i>{{ item.java_major }}</span>
          <span><i>安装于</i>{{ formatDate(new Date(item.last_modified * 1000).toISOString()) }}</span>
        </div>

        <div class="version-row__actions">
          <IconButton
            icon="play"
            :label="`启动 ${item.id}`"
            variant="primary"
            :size="30"
            @click="selectVersion(item.id); launchVersion(item.id)"
          />
          <IconButton
            icon="check"
            label="设为当前版本"
            :size="30"
            :disabled="item.id === selectedVersion?.id"
            @click="selectVersion(item.id)"
          />
          <IconButton
            icon="settings"
            label="版本设置"
            variant="soft"
            :size="30"
            @click="openInstance(item.id)"
          />
          <IconButton
            icon="trash"
            :label="
              confirmId === item.id
                ? '确认删除'
                : store.metas[item.id]?.category === 'modpack'
                  ? '删除整合包'
                  : '删除版本'
            "
            variant="danger-quiet"
            :size="30"
            @click="requestDelete(item.id)"
          />
        </div>
      </article>
    </div>
  </div>
</template>

<style scoped>
.blank {
  margin-top: 18vh;
  text-align: center;
  user-select: none;
}

.dir-switch {
  position: relative;
  display: flex;
}

.dir-backdrop {
  position: fixed;
  inset: 0;
  z-index: 20;
}

.dir-panel {
  position: absolute;
  right: 0;
  top: calc(100% + 10px);
  z-index: 21;
  width: 320px;
  padding: 8px;
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 2px;
  background: var(--panel);
  box-shadow: 0 14px 34px rgba(52, 38, 24, 0.22);
}

.dir-panel__title {
  display: block;
  padding: 6px 8px 8px;
  font-family: var(--font-mono);
  font-size: 9.5px;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--muted);
}

.dir-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 9px 10px;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition:
    background var(--dur-2) ease,
    border-color var(--dur-2) ease;
}

.dir-item:hover {
  background: rgba(var(--accent-rgb), 0.06);
}

.dir-item--active {
  border-color: var(--moss);
  background: rgba(var(--accent-rgb), 0.08);
}

.dir-item__body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.dir-item__body strong {
  font-size: 13px;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dir-item__path {
  font-size: 9.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dir-item__count {
  flex-shrink: 0;
  font-size: 10.5px;
  color: var(--muted);
}

.dir-manage {
  display: block;
  width: 100%;
  margin-top: 4px;
  padding: 8px 0;
  border: none;
  border-top: 1px dashed var(--line);
  border-radius: 0 0 8px 8px;
  background: transparent;
  font-size: 11.5px;
  color: var(--ink-soft);
  cursor: pointer;
  transition: color var(--dur-2) ease;
}

.dir-manage:hover {
  color: var(--moss);
}

.blank__title {
  font-size: 19px;
  letter-spacing: 0.18em;
  color: var(--ink-soft);
}

.blank__hint {
  margin-top: 10px;
  font-size: 12.5px;
  letter-spacing: 0.06em;
  color: var(--muted);
}

.page-head--split {
  justify-content: space-between;
  align-items: flex-end;
}

.page-head__lead {
  min-width: 0;
}

/* ---------- 本地游玩画像 ---------- */
.shelf-stats {
  --profile-ink: #fff4e2;
  --profile-soft: rgba(255, 244, 226, 0.8);
  --profile-muted: rgba(255, 244, 226, 0.55);
  --profile-shadow: 0 1px 4px rgba(20, 14, 8, 0.62);
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(260px, 300px);
  gap: 10px;
  align-items: stretch;
  text-shadow: var(--profile-shadow);
}

.shelf-hero {
  position: relative;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 22px;
  min-height: 300px;
  padding: 24px 26px 20px;
  overflow: hidden;
  border: 2px solid var(--frame);
  border-radius: 2px;
  background-color: rgba(var(--glass-rgb), 0.92);
  background-image: var(--hero-bg);
  background-position: center;
  background-size: cover;
  box-shadow: var(--shadow-1);
}

.shelf-hero--empty {
  justify-content: center;
}

.hero-bottom {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.hero-total {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

.hero-total__eyebrow {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  font-weight: 800;
  letter-spacing: 0.04em;
  color: var(--profile-ink);
}

.hero-total__eyebrow::before {
  content: "";
  width: 18px;
  height: 3px;
  border-radius: 99px;
  background: var(--moss);
}

.hero-total__num {
  display: flex;
  align-items: baseline;
  gap: 5px;
  font-family: var(--font-display);
  color: var(--profile-ink);
  line-height: 1;
}

.hero-total__num strong {
  font-size: 42px;
  font-weight: 700;
  letter-spacing: -0.035em;
}

.hero-total__unit {
  margin-right: 6px;
  font-size: 12.5px;
  color: var(--profile-soft);
  letter-spacing: 0.02em;
}

.hero-total__num--empty {
  padding: 12px 0 6px;
  font-family: var(--font-ui);
  font-size: 17px;
  font-weight: 600;
  color: var(--profile-muted);
}

.hero-facts {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 18px;
  margin-top: 5px;
}

.hero-facts span {
  display: inline-flex;
  align-items: baseline;
  gap: 4px;
  font-size: 11.5px;
  color: var(--profile-soft);
}

.hero-facts b {
  font-family: var(--font-display);
  font-size: 13px;
  color: var(--profile-ink);
}

.hero-recent {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 8px;
  margin-top: 6px;
  font-size: 11.5px;
  color: var(--profile-soft);
}

.hero-recent__label {
  font-family: var(--font-mono);
  font-size: 9.5px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--profile-muted);
}

.hero-recent strong {
  font-family: var(--font-display);
  font-size: 13px;
  color: var(--profile-ink);
}

.hero-recent__meta {
  color: var(--profile-soft);
}

.week {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
  /* 不铺满整卡，右侧留白 */
  max-width: 440px;
}

.week__title {
  display: block;
  font-size: 12.5px;
  font-weight: 800;
  color: var(--profile-ink);
}

.week__bars {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 8px;
  align-items: end;
}

.week__col {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  min-width: 0;
}

.week__track {
  display: flex;
  align-items: flex-end;
  justify-content: center;
  width: 100%;
  height: 28px;
  border-bottom: 2px solid var(--profile-muted);
}

.week__bar {
  display: block;
  width: min(38px, 78%);
  border-radius: 0;
  background: linear-gradient(180deg, var(--moss-bright), var(--moss));
  transition: height var(--dur-3) var(--ease-out-expo);
}

.week__bar--empty {
  background: rgba(var(--accent-rgb), 0.14);
}

.week__bar--today {
  background: linear-gradient(
    180deg,
    var(--amber),
    color-mix(in srgb, var(--amber) 68%, var(--walnut))
  );
}

.week__label {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.04em;
  color: var(--profile-soft);
}

.week__label--today {
  font-weight: 700;
  color: var(--profile-ink);
}

.hero-dist {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
  /* 比柱状图略宽，两条长度错开 */
  max-width: 490px;
}

.hero-dist__bar {
  display: flex;
  height: 8px;
  overflow: hidden;
  border-radius: 0;
  background: rgba(var(--accent-rgb), 0.08);
}

.hero-dist__bar span {
  height: 100%;
  transition: width var(--dur-4) var(--ease-out-expo);
}

.hero-dist__bar span:not(:last-child) {
  box-shadow: 2px 0 0 rgba(24, 17, 10, 0.42);
}

.hero-dist__legend {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
}

.hero-dist__item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 11.5px;
  color: var(--profile-soft);
}

.hero-dist__item i {
  flex-shrink: 0;
  width: 9px;
  height: 9px;
  border-radius: 0;
}

.hero-dist__item b {
  overflow: hidden;
  max-width: 110px;
  font-weight: 600;
  color: var(--profile-ink);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hero-dist__item em {
  font-style: normal;
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--profile-muted);
}

.shelf-side {
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  gap: 10px;
  min-width: 0;
}

/* 「最常玩版本」上方留白里放一颗会转的玩家头颅，和启动台同一个组件 */
.shelf-head {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  overflow: hidden;
}

.stat-card {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  overflow: hidden;
  padding: 13px 15px;
  border: 2px solid var(--frame);
  border-radius: 2px;
  background-color: rgba(var(--glass-rgb), 0.86);
  box-shadow: var(--shadow-1);
}

.stat-card--top {
  flex: none;
  justify-content: flex-start;
  gap: 10px;
  min-height: 200px;
  padding: 18px 20px 16px;
  background-image: var(--card-bg);
  background-position: center;
  background-size: cover;
}

.stat-card__label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--profile-soft);
}

.stat-card__label::before {
  content: "";
  flex-shrink: 0;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--moss);
}

.stat-empty {
  padding: 6px 0;
  font-size: 12px;
  color: var(--profile-muted);
}

.top-overview {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 14px;
  padding-top: 10px;
  border-top: 1px solid var(--profile-muted);
  font-size: 11px;
  color: var(--profile-soft);
}

.top-overview b {
  font-family: var(--font-display);
  font-size: 13px;
  color: var(--profile-ink);
}

.top-version {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 10px;
  min-width: 0;
  /* 贴底、靠左：图标放到版本名侧边，不再悬在卡片正中 */
  margin: auto 0 0;
}

.top-version__tile {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: 38px;
  height: 38px;
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: rgba(var(--accent-rgb), 0.08);
}

.top-version__tile img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.top-version__body {
  display: flex;
  flex: 0 1 auto;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  max-width: 190px;
}

.top-version__body strong {
  overflow: hidden;
  font-family: var(--font-display);
  font-size: 21px;
  color: var(--profile-ink);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.version-row__share {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 3px;
}

.version-row__share-track {
  flex: 1;
  max-width: 240px;
  height: 4px;
  overflow: hidden;
  border-radius: 1px;
  background: rgba(var(--accent-rgb), 0.12);
}

.version-row__share-track span {
  display: block;
  height: 100%;
  border-radius: 1px;
  background: var(--moss);
  box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.2);
  transition: width var(--dur-4) var(--ease-out-expo);
}

.version-row__share-num {
  flex-shrink: 0;
  font-size: 9.5px;
  letter-spacing: 0.04em;
  color: var(--muted);
}

.version-row__play {
  display: flex;
  flex-shrink: 0;
  flex-direction: column;
  align-items: flex-end;
  gap: 1px;
  min-width: 78px;
}

.version-row__play strong {
  font-family: var(--font-display);
  font-size: 14.5px;
  color: var(--walnut-deep);
  white-space: nowrap;
}

.version-row__play span {
  font-family: var(--font-mono);
  font-size: 9.5px;
  color: var(--muted);
  white-space: nowrap;
}

.version-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 8px;
  /* 版本列表是一只实体托盘，行本身保持无框、靠悬停着色 */
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 2px;
  /* 整块浅色面板兜底，毛玻璃/无底图下行内容也能看清 */
  background: rgba(var(--glass-rgb), 0.88);
}

/* PCL 式紧凑行：默认无框透明，悬停/选中才铺浅色 */
.version-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px 8px 16px;
  overflow: visible;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  transition: background var(--dur-2) ease;
}

.version-row:hover {
  background: rgba(var(--accent-rgb), 0.05);
}

.version-row--active {
  background: rgba(var(--accent-rgb), 0.08);
}

.version-row::after {
  content: "";
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: transparent;
  transition: background var(--dur-2) ease;
}

.version-row--active::after {
  background: var(--moss);
}

.version-row--picking {
  z-index: 6;
}

.version-row__icon-wrap {
  position: relative;
  flex-shrink: 0;
}

.version-row__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  padding: 0;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: rgba(var(--accent-rgb), 0.05);
  cursor: pointer;
  overflow: hidden;
  transition:
    transform var(--dur-2) ease,
    border-color var(--dur-2) ease,
    box-shadow var(--dur-2) ease;
}

.version-row__icon:hover {
  transform: translateY(-1px);
  border-color: var(--moss);
  box-shadow: 0 4px 12px rgba(var(--accent-rgb), 0.16);
}

.version-row__icon img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.version-row__main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1 1 240px;
}

.version-row__title {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.version-row__title strong {
  font-size: 15.5px;
  line-height: 1.25;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.version-row__sub {
  font-size: 10.5px;
  color: var(--muted);
  letter-spacing: 0.04em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.version-row__meta {
  display: flex;
  flex: 0 0 auto;
  flex-wrap: wrap;
  gap: 4px 16px;
  color: var(--ink-soft);
  font-size: 12px;
  white-space: nowrap;
}

.version-row__meta i {
  font-style: normal;
  font-family: var(--font-mono);
  font-size: 9px;
  letter-spacing: 0.1em;
  color: var(--muted);
  margin-right: 5px;
  text-transform: uppercase;
}

.version-row__actions {
  display: flex;
  flex-shrink: 0;
  gap: 2px;
  opacity: 0;
  transition: opacity var(--dur-2) ease;
}

.version-row:hover .version-row__actions,
.version-row:focus-within .version-row__actions,
.version-row--picking .version-row__actions {
  opacity: 1;
}

/* PCL 式浅显图标：去边框去底色，只留线条图案 */
.version-row__actions :deep(.icon-btn--primary),
.version-row__actions :deep(.icon-btn--soft),
.version-row__actions :deep(.icon-btn--danger) {
  border-color: transparent;
  background: transparent;
  box-shadow: none;
}

.version-row__actions :deep(.icon-btn--primary) {
  color: var(--moss);
}

.version-row__actions :deep(.icon-btn--danger) {
  color: var(--danger);
}

.version-row__actions :deep(.icon-btn--primary:hover:not(:disabled)),
.version-row__actions :deep(.icon-btn--soft:hover:not(:disabled)) {
  border-color: transparent;
  background: rgba(var(--accent-rgb), 0.08);
  box-shadow: none;
  color: var(--walnut-deep);
}

.version-row__actions :deep(.icon-btn--danger:hover:not(:disabled)) {
  border-color: transparent;
  background: color-mix(in srgb, var(--danger) 12%, transparent);
  box-shadow: none;
  color: var(--danger);
}

.picker-backdrop {
  position: fixed;
  inset: 0;
  z-index: 20;
}

.icon-picker {
  position: absolute;
  left: 46px;
  top: 46px;
  z-index: 21;
  width: 268px;
  padding: 12px;
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 2px;
  background: var(--panel);
  box-shadow: 0 14px 34px rgba(52, 38, 24, 0.22);
}

.icon-picker__title {
  display: block;
  margin-bottom: 8px;
  font-family: var(--font-mono);
  font-size: 9.5px;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--muted);
}

.icon-picker__grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
}

.icon-picker__chip {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 6px 2px;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  font-size: 10.5px;
  color: var(--ink-soft);
  cursor: pointer;
  transition:
    background var(--dur-2) ease,
    border-color var(--dur-2) ease;
}

.icon-picker__chip:hover {
  background: rgba(var(--accent-rgb), 0.06);
}

.icon-picker__chip--active {
  border-color: var(--moss);
  background: rgba(var(--accent-rgb), 0.08);
}

.icon-picker__tile {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: 0;
  background: color-mix(in srgb, var(--tile) 18%, rgba(255, 255, 255, 0.6));
}

.icon-picker__upload {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  margin-top: 10px;
  padding: 7px 0;
  border: 1px dashed var(--line);
  border-radius: 4px;
  background: transparent;
  font-size: 11.5px;
  color: var(--ink-soft);
  cursor: pointer;
  transition:
    color var(--dur-2) ease,
    border-color var(--dur-2) ease;
}

.icon-picker__upload:hover {
  color: var(--moss);
  border-color: var(--moss);
}

@media (max-width: 1180px) {
  .version-row__meta {
    display: none;
  }
}

@media (max-width: 1080px) {
  .shelf-stats {
    grid-template-columns: minmax(0, 1fr);
  }

  .shelf-hero {
    min-height: 0;
  }

  /* 竖排后没有那块留白了，头颅一起收起 */
  .shelf-head {
    display: none;
  }

  .stat-card--top {
    min-height: 0;
  }
}
</style>
