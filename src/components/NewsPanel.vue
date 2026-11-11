<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { api, openExternal, type CfProject, type NewsItem } from "../api";
import AppIcon from "./AppIcon.vue";
import IconButton from "./IconButton.vue";
import WoodButton from "./WoodButton.vue";
import WoodModal from "./WoodModal.vue";
import { notify, store } from "../store";
import { formatDate } from "../utils";

type Tab = "all" | "official" | "patch" | "community";

const tabs: { value: Tab; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "official", label: "官方" },
  { value: "patch", label: "版本" },
  { value: "community", label: "社区" },
];

const official = ref<NewsItem[]>([]);
const community = ref<NewsItem[]>([]);
const loading = ref(true);
const refreshing = ref(false);
const communityLoading = ref(false);
const stale = ref(false);
const errors = ref<string[]>([]);
const updatedAt = ref(0);
const tab = ref<Tab>("all");
const expanded = ref(localStorage.getItem("docl.news.expanded") === "1");
const viewMode = ref<"grid" | "list">(
  localStorage.getItem("docl.news.view") === "list" ? "list" : "grid"
);
const brokenImages = reactive(new Set<string>());
const article = ref<{ item: NewsItem; text: string } | null>(null);
const articleLoading = ref(false);
const communityOffset = ref(0);
const communityDone = ref(false);
let lastRefresh = 0;

const merged = computed(() => {
  const list = [...official.value, ...community.value];
  list.sort((a, b) => (b.date || "").localeCompare(a.date || ""));
  return list;
});

const filtered = computed(() =>
  tab.value === "all" ? merged.value : merged.value.filter((item) => item.source === tab.value)
);

const preview = computed(() => filtered.value.slice(0, 2));

const updatedLabel = computed(() => {
  if (!updatedAt.value) return "尚未更新";
  const date = new Date(updatedAt.value * 1000);
  return `更新于 ${String(date.getHours()).padStart(2, "0")}:${String(
    date.getMinutes()
  ).padStart(2, "0")}`;
});

function shortDate(value: string) {
  if (!value) return "—";
  const formatted = formatDate(value);
  return formatted === "—" ? value.slice(0, 10) : formatted;
}

function tagClass(item: NewsItem) {
  if (item.source === "patch") return "tag--grass";
  if (item.source === "community") return "news-tag--community";
  return "";
}

function toggle() {
  expanded.value = !expanded.value;
  localStorage.setItem("docl.news.expanded", expanded.value ? "1" : "0");
}

function toggleView() {
  viewMode.value = viewMode.value === "grid" ? "list" : "grid";
  localStorage.setItem("docl.news.view", viewMode.value);
}

async function loadOfficial(force = false) {
  refreshing.value = true;
  try {
    const feed = await api.newsFeed(force);
    official.value = feed.items;
    stale.value = feed.stale;
    errors.value = feed.errors;
    updatedAt.value = feed.updated_at;
  } catch (err) {
    errors.value = [String(err)];
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

function mapCommunity(items: CfProject[]): NewsItem[] {
  return items.map((item) => ({
    id: item.id,
    title: item.name,
    category: "整合包",
    tag: "MODRINTH",
    date: (item.updated ?? "").slice(0, 10),
    summary: item.summary,
    image_url: item.logo_url || null,
    url: null,
    source: "community",
    version: null,
    kind: null,
    detail_path: null,
  }));
}

async function loadCommunity(reset = false) {
  if (communityLoading.value) return;
  if (reset) {
    communityOffset.value = 0;
    communityDone.value = false;
  }
  if (communityDone.value) return;
  communityLoading.value = true;
  try {
    const result = await api.modrinthSearch(
      "modpacks",
      "",
      null,
      null,
      communityOffset.value,
      3
    );
    const fresh = mapCommunity(result.items);
    const seen = new Set(community.value.map((item) => item.id));
    const mergedList = reset
      ? fresh
      : [...community.value, ...fresh.filter((item) => !seen.has(item.id))];
    community.value = mergedList;
    communityOffset.value += result.items.length;
    if (!result.items.length || mergedList.length >= result.total) {
      communityDone.value = true;
    }
  } catch {
    if (reset) community.value = [];
  } finally {
    communityLoading.value = false;
  }
}

watch(tab, (value) => {
  if (value === "community" && !community.value.length && !communityLoading.value) {
    void loadCommunity(true);
  }
});

function refresh() {
  const now = Date.now();
  if (now - lastRefresh < 3000) return;
  lastRefresh = now;
  void loadOfficial(true);
  void loadCommunity(true);
}

async function openItem(item: NewsItem) {
  if (item.source === "community") {
    store.communityContext = {
      kind: "modpacks",
      instanceId: "",
      gameVersion: "",
      loader: "",
      query: item.title,
      projectId: item.id,
    };
    store.page = "download";
    return;
  }
  if (item.source === "patch" && item.detail_path) {
    article.value = { item, text: "" };
    articleLoading.value = true;
    try {
      const wikiTitle = item.version ? `Java版${item.version}` : undefined;
      const text = await api.newsArticle(item.detail_path, wikiTitle);
      if (article.value) article.value.text = text;
    } catch (err) {
      article.value = null;
      notify(`读取更新日志失败：${String(err)}`, "err");
    } finally {
      articleLoading.value = false;
    }
    return;
  }
  if (item.url) {
    try {
      await openExternal(item.url);
    } catch (err) {
      notify(`打开链接失败：${String(err)}`, "err");
    }
  }
}

function openArticleLink() {
  const url = article.value?.item.url;
  if (!url) return;
  void openExternal(url).catch((err) => notify(`打开链接失败：${String(err)}`, "err"));
}

onMounted(() => {
  void loadOfficial(false);
  void loadCommunity(true);
});
</script>

<template>
  <section class="news card reveal" :style="{ '--i': 1 }">
    <header class="news__head">
      <div class="news__heading">
        <div class="news__heading-row">
          <span class="section-title">动态</span>
          <span v-if="stale" class="tag news-tag--stale">离线缓存</span>
        </div>
      </div>

      <div class="news__tabs">
        <button
          v-for="item in tabs"
          :key="item.value"
          class="news__tab"
          :class="{ 'news__tab--active': tab === item.value }"
          @click="tab = item.value"
        >
          {{ item.label }}
        </button>
      </div>

      <div class="news__actions">
        <IconButton
          v-if="expanded"
          :icon="viewMode === 'grid' ? 'grid' : 'list'"
          :label="viewMode === 'grid' ? '卡片视图（点击切换为列表）' : '列表视图（点击切换为卡片）'"
          :size="30"
          @click="toggleView"
        />
        <IconButton
          icon="refresh"
          label="刷新动态"
          :size="30"
          :disabled="refreshing"
          @click="refresh"
        />
        <IconButton
          :icon="expanded ? 'chevron-up' : 'chevron-down'"
          :label="expanded ? '收起动态' : '展开动态'"
          :size="30"
          :active="expanded"
          @click="toggle"
        />
      </div>
    </header>

    <div
      v-if="loading || (tab === 'community' && communityLoading && !community.length)"
      class="news__skeleton"
    >
      <span v-for="index in 2" :key="index" class="skeleton news__skeleton-row" />
    </div>

    <div v-else-if="!filtered.length" class="news__empty">
      <span>{{ errors.length ? errors[0] : "这个分类暂时没有内容" }}</span>
      <WoodButton variant="ghost" size="sm" @click="refresh">重试</WoodButton>
    </div>

    <div v-else class="news__body" :class="{ 'news__body--open': expanded }">
      <div class="news__body-inner">
        <div v-if="!expanded" class="news__collapsed">
          <button
            v-for="item in preview"
            :key="`${item.source}-${item.id}`"
            class="news-row"
            @click="openItem(item)"
          >
            <span class="news-row__date mono">{{ shortDate(item.date) }}</span>
            <span v-if="item.source !== 'community'" class="tag" :class="tagClass(item)">
              {{ item.tag }}
            </span>
            <span class="news-row__title">{{ item.title }}</span>
          </button>
          <button class="news__more" @click="toggle">
            展开全部 {{ filtered.length }} 条
            <AppIcon name="chevron-down" :size="14" />
          </button>
        </div>

        <div v-else-if="viewMode === 'grid'" class="news__grid">
          <button
            v-for="item in filtered"
            :key="`${item.source}-${item.id}`"
            class="news-card"
            @click="openItem(item)"
          >
            <span v-if="item.image_url && !brokenImages.has(item.id)" class="news-card__media">
              <img
                :src="item.image_url"
                alt=""
                loading="lazy"
                @error="brokenImages.add(item.id)"
              />
            </span>
            <span class="news-card__meta">
              <span class="tag" :class="tagClass(item)">{{ item.tag }}</span>
              <span class="mono">{{ shortDate(item.date) }}</span>
            </span>
            <strong class="news-card__title">{{ item.title }}</strong>
            <span class="news-card__summary">{{ item.summary }}</span>
          </button>
        </div>

        <div v-else class="news__list">
          <button
            v-for="item in filtered"
            :key="`${item.source}-${item.id}`"
            class="news-row news-row--rich"
            @click="openItem(item)"
          >
            <span v-if="item.image_url && !brokenImages.has(item.id)" class="news-row__thumb">
              <img
                :src="item.image_url"
                alt=""
                loading="lazy"
                @error="brokenImages.add(item.id)"
              />
            </span>
            <span class="news-row__date mono">{{ shortDate(item.date) }}</span>
            <span v-if="item.source !== 'community'" class="tag" :class="tagClass(item)">
              {{ item.tag }}
            </span>
            <span class="news-row__text">
              <span class="news-row__title">{{ item.title }}</span>
              <span class="news-row__summary">{{ item.summary }}</span>
            </span>
          </button>
        </div>

        <div class="news__foot">
          <span class="muted small">{{ updatedLabel }}</span>
          <button
            v-if="expanded && tab === 'community' && !communityDone"
            class="news__more"
            :disabled="communityLoading"
            @click="loadCommunity()"
          >
            {{ communityLoading ? "加载中…" : "加载更多整合包" }}
            <AppIcon v-if="!communityLoading" name="chevron-down" :size="13" />
          </button>
          <button v-if="expanded" class="news__more" @click="toggle">
            收起
            <AppIcon name="chevron-up" :size="13" />
          </button>
        </div>
      </div>
    </div>

    <WoodModal
      v-if="article"
      :title="article.item.title"
      :width="640"
      @close="article = null"
    >
      <div class="news-article-meta">
        <span class="tag tag--grass">{{ article.item.tag }}</span>
        <span class="mono muted">{{ shortDate(article.item.date) }}</span>
        <span v-if="article.item.version" class="mono muted">
          {{ article.item.version }}
        </span>
      </div>
      <div v-if="articleLoading" class="news-article-loading">
        <span class="skeleton news__skeleton-row" />
        <span class="skeleton news__skeleton-row" />
        <span class="skeleton news__skeleton-row" />
      </div>
      <pre v-if="!articleLoading" class="news-article">{{ article.text }}</pre>
      <div v-if="article.item.url && !articleLoading" class="news-article-actions">
        <WoodButton variant="ghost" size="sm" @click="openArticleLink">
          <AppIcon name="globe" :size="13" />
          在中文 Wiki 查看汉化介绍
        </WoodButton>
      </div>
    </WoodModal>
  </section>
</template>

<style scoped>
.news {
  padding: 18px 20px 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.news__head {
  display: flex;
  align-items: center;
  gap: 18px;
  flex-wrap: wrap;
}

.news__heading {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.news__heading-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.news-tag--stale {
  background: var(--panel-2);
  color: var(--muted);
  border-color: var(--line-strong);
}

.news-tag--community {
  background: rgba(217, 160, 91, 0.14);
  color: var(--walnut);
  border-color: rgba(217, 160, 91, 0.35);
}

/* 分段控件：外层是浅凹槽，选中项像按键一样被压进去 */
.news__tabs {
  display: flex;
  gap: 2px;
  padding: 3px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  box-shadow: inset 0 1px 2px rgba(40, 28, 16, 0.08);
  margin-left: auto;
}

.news__tab {
  --face: var(--panel);
  height: 27px;
  padding: 0 12px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  color: var(--ink-soft);
  font-size: 12px;
  font-weight: 600;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    border-color var(--dur-1) ease, transform var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.news__tab:not(.news__tab--active):hover {
  color: var(--walnut-deep);
  background: rgba(var(--accent-rgb), 0.07);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 72%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 72%, #000);
}

.news__tab:active,
.news__tab--active {
  background: var(--face);
  color: var(--walnut-deep);
  font-weight: 700;
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 72%, #000);
  border-left-color: color-mix(in srgb, var(--face) 72%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.16);
}

.news__actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.news__skeleton {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.news__skeleton-row {
  display: block;
  height: 38px;
  border-radius: 4px;
}

.news__empty {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 4px;
  color: var(--muted);
  font-size: 12.5px;
}

.news__body {
  max-height: 172px;
  overflow: hidden;
  transition: max-height 420ms var(--ease-out-expo);
}

/* 展开态：内容在动态卡片内部滚动，超出部分不再被外层裁掉 */
.news__body--open {
  flex: 1;
  min-height: 0;
  max-height: none;
  overflow-y: auto;
}

.news__body-inner {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.news__collapsed {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.news-row {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 38px;
  padding: 0 8px;
  border-radius: 0;
  text-align: left;
  color: var(--ink);
  transition: background var(--dur-1) ease, box-shadow var(--dur-1) ease;
}

.news-row:hover {
  background: var(--hover);
  box-shadow: inset 3px 0 0 var(--moss);
}

.news-row__date {
  font-size: 10.5px;
  color: var(--muted);
  flex-shrink: 0;
}

.news-row__title {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.news__more {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  align-self: flex-start;
  height: 28px;
  padding: 0 8px;
  border-radius: 3px;
  color: var(--moss);
  font-size: 12px;
  font-weight: 600;
}

.news__more:hover {
  background: var(--moss-soft);
}

.news__more:disabled {
  opacity: 0.6;
  cursor: default;
}

.news__list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-top: 2px;
}

.news-row--rich {
  height: auto;
  padding: 8px;
  align-items: center;
}

.news-row__thumb {
  width: 40px;
  height: 40px;
  border-radius: 0;
  overflow: hidden;
  flex-shrink: 0;
  background: var(--paper-2);
  border: 1px solid var(--line);
}

.news-row__thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.news-row__text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.news-row--rich .news-row__title {
  flex: none;
}

.news-row__summary {
  font-size: 11.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.news__grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  padding-top: 2px;
}

.news-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px;
  border-style: solid;
  border-width: 2px;
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 0;
  background: var(--panel-2);
  text-align: left;
  overflow: hidden;
  transition: border-color var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo),
    box-shadow var(--dur-2) ease;
}

.news-card:hover {
  transform: translateY(-2px);
  border-top-color: color-mix(in srgb, var(--frame) 78%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 78%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 86%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 86%, #000);
  box-shadow: var(--shadow-1);
}

.news-card__media {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  border-radius: 0;
  overflow: hidden;
  background: var(--paper-2);
  border: 1px solid var(--line);
}

.news-card__media img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.news-card__meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10.5px;
  color: var(--muted);
}

.news-card__title {
  font-size: 13.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  line-height: 1.35;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.news-card__summary {
  font-size: 12px;
  color: var(--ink-soft);
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.news__foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 2px;
}

.news-article-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
  font-size: 11.5px;
}

.news-article-loading {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.news-article-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 10px;
}

.news-article {
  margin: 0;
  max-height: 56vh;
  overflow-y: auto;
  font-family: var(--font-ui);
  font-size: 13px;
  line-height: 1.7;
  color: var(--ink-soft);
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
}

@media (max-width: 900px) {
  .news__grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .news__tabs {
    margin-left: 0;
  }
}
</style>
