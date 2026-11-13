<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { api, type LoaderOption, type LoaderVersion } from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodProgress from "../components/WoodProgress.vue";
import CommunityView from "./CommunityView.vue";
import {
  cancelInstall,
  installLoader,
  installVersion,
  launchVersion,
  notify,
  refreshVersions,
  store,
} from "../store";
import {
  APRIL_FOOLS_IDS,
  formatBytes,
  formatDate,
  formatSpeed,
  kindLabel,
  matchVersionFilter,
  type VersionFilter,
} from "../utils";

type Category =
  | "vanilla"
  | "mods"
  | "modpacks"
  | "datapacks"
  | "worlds"
  | "resourcepacks"
  | "shaders";

const categories: { value: Category; label: string; icon: string }[] = [
  { value: "vanilla", label: "原版游戏", icon: "cube" },
  { value: "mods", label: "Mod", icon: "grid" },
  { value: "modpacks", label: "整合包", icon: "package" },
  { value: "datapacks", label: "数据包", icon: "database" },
  { value: "worlds", label: "地图", icon: "map" },
  { value: "resourcepacks", label: "资源包", icon: "image" },
  { value: "shaders", label: "光影包", icon: "sun" },
];

const category = ref<Category>("vanilla");
const detailId = ref<string | null>(null);
const loaders = ref<LoaderOption[]>([]);
const loadersLoading = ref(false);
const expandedLoader = ref<string | null>(null);
const loaderVersionMap = ref<Record<string, LoaderVersion[]>>({});
const versionLoading = ref<string | null>(null);
const versionError = ref("");

const filter = ref<VersionFilter>("release");
const query = ref("");
const limit = ref(240);

const tabs = [
  { key: "release", label: "正式版" },
  { key: "snapshot", label: "快照版" },
  { key: "april", label: "愚人节版" },
  { key: "legacy", label: "远古版" },
] as const;

onMounted(() => {
  const context = store.communityContext;
  if (context && categories.some((item) => item.value === context.kind)) {
    category.value = context.kind as Category;
  }
  if (!store.versions.length) refreshVersions();
  if (import.meta.env.DEV) window.addEventListener("docl-dev-dl-category", onDevCategory);
});

onBeforeUnmount(() => {
  window.removeEventListener("docl-dev-dl-category", onDevCategory);
});

/// 截图脚本用：直接切到指定分类
function onDevCategory(event: Event) {
  switchCategory((event as CustomEvent<Category>).detail);
}

watch(detailId, async (id) => {
  loaders.value = [];
  expandedLoader.value = null;
  loaderVersionMap.value = {};
  versionError.value = "";
  if (!id) return;
  loadersLoading.value = true;
  try {
    loaders.value = await api.loaderOptions(id);
  } catch (err) {
    notify(`获取加载器列表失败：${String(err)}`, "err");
  } finally {
    loadersLoading.value = false;
  }
});

watch(
  () => store.installed.length,
  async () => {
    if (!detailId.value || loadersLoading.value) return;
    try {
      loaders.value = await api.loaderOptions(detailId.value);
    } catch {
      // 忽略刷新失败，保留原有列表
    }
  }
);

const detailSummary = computed(() =>
  store.versions.find((v) => v.id === detailId.value)
);

const detailInstalled = computed(
  () => !!store.installed.find((v) => v.id === detailId.value)
);

const filtered = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  return store.versions
    .filter((v) => matchVersionFilter(v.kind, v.id, filter.value))
    .filter((v) => (keyword ? v.id.toLowerCase().includes(keyword) : true))
    .slice(0, limit.value);
});

const totalMatched = computed(() => {
  const keyword = query.value.trim().toLowerCase();
  return store.versions
    .filter((v) => matchVersionFilter(v.kind, v.id, filter.value))
    .filter((v) => (keyword ? v.id.toLowerCase().includes(keyword) : true)).length;
});

const progress = computed(() => {
  if (!store.installingId || !store.progress) return null;
  if (store.progress.stage === "java") return null;
  return store.progress;
});

const percent = computed(() => {
  const p = progress.value;
  if (!p) return 0;
  if (p.bytes_total > 0) return Math.min(100, (p.bytes_done / p.bytes_total) * 100);
  if (p.files_total > 0) return Math.min(100, (p.files_done / p.files_total) * 100);
  return 0;
});

function switchCategory(value: Category) {
  category.value = value;
  detailId.value = null;
}

function openDetail(id: string) {
  detailId.value = id;
}

function backToList() {
  detailId.value = null;
}

function installVanilla() {
  if (detailId.value) installVersion(detailId.value);
}

function launchVanilla() {
  if (detailId.value) launchVersion(detailId.value);
}

function installLoaderFor(loader: string, loaderVersion?: string) {
  if (detailId.value) installLoader(detailId.value, loader, loaderVersion);
}

async function toggleLoader(loader: string) {
  if (expandedLoader.value === loader) {
    expandedLoader.value = null;
    return;
  }
  expandedLoader.value = loader;
  versionError.value = "";
  if (loaderVersionMap.value[loader]) return;
  if (!detailId.value) return;
  versionLoading.value = loader;
  try {
    const list = await api.loaderVersions(detailId.value, loader);
    loaderVersionMap.value = {
      ...loaderVersionMap.value,
      [loader]: list,
    };
  } catch (err) {
    versionError.value = String(err);
  } finally {
    versionLoading.value = null;
  }
}

function launchLoader(id: string | null) {
  if (id) launchVersion(id);
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div class="page-head__actions">
        <IconButton
          v-if="category === 'vanilla' && !detailId"
          icon="refresh"
          label="刷新版本列表"
          variant="soft"
          :size="32"
          @click="refreshVersions(true)"
        />
      </div>
    </header>

    <div class="download-layout">
      <aside class="dl-side card">
        <button
          v-for="item in categories.slice(0, 1)"
          :key="item.value"
          class="dl-side__item"
          :class="{ 'dl-side__item--active': category === item.value }"
          @click="switchCategory(item.value)"
        >
          <AppIcon :name="item.icon" :size="17" />
          <span>{{ item.label }}</span>
        </button>

        <div class="dl-side__label">社区资源</div>
        <button
          v-for="item in categories.slice(1)"
          :key="item.value"
          class="dl-side__item"
          :class="{ 'dl-side__item--active': category === item.value }"
          @click="switchCategory(item.value)"
        >
          <AppIcon :name="item.icon" :size="17" />
          <span>{{ item.label }}</span>
        </button>
      </aside>

      <div class="dl-main">
        <template v-if="category !== 'vanilla'">
          <CommunityView :key="category" :kind="category" />
        </template>

        <template v-else-if="detailId">
          <div class="detail-head">
            <IconButton
              icon="chevron-left"
              label="返回版本列表"
              :size="34"
              @click="backToList"
            />
            <div class="detail-head__text">
              <span class="section-title">原版版本</span>
              <h2 class="detail-title display">{{ detailId }}</h2>
            </div>
            <div class="row detail-head__tags">
              <span class="tag">{{ kindLabel(detailSummary?.kind ?? "release") }}</span>
              <span v-if="detailInstalled" class="tag tag--grass">已安装</span>
            </div>
          </div>

          <section class="card card-pad vanilla-row">
            <div>
              <div class="section-title">原版客户端</div>
              <p class="muted small">
                安装基础版本后，才能在其上添加 Fabric、Quilt 等加载器。
              </p>
            </div>
            <div class="row">
              <WoodButton
                v-if="!detailInstalled"
                variant="primary"
                sheen
                :disabled="!!store.installingId"
                @click="installVanilla"
              >
                <AppIcon name="download" :size="15" />
                安装原版
              </WoodButton>
              <WoodButton
                v-else
                variant="primary"
                sheen
                :disabled="!!store.installingId"
                @click="launchVanilla"
              >
                <AppIcon name="play" :size="15" />
                启动原版
              </WoodButton>
            </div>
          </section>

          <section class="card card-pad loader-panel">
            <div class="row-between">
              <div>
                <div class="section-title">可添加的加载器</div>
                <p class="muted small">安装后会生成独立的版本条目，可单独启动。</p>
              </div>
            </div>

            <div v-if="loadersLoading" class="loader-list">
              <div v-for="n in 4" :key="n" class="loader-row loader-row--skeleton">
                <span class="skeleton loader-skeleton__badge" />
                <span class="skeleton loader-skeleton__text" />
                <span class="skeleton loader-skeleton__btn" />
              </div>
            </div>

            <div v-else class="loader-list">
              <template v-for="item in loaders" :key="item.loader">
                <div
                  class="loader-row"
                  :class="{
                    'loader-row--installed': item.installed,
                    'loader-row--open': expandedLoader === item.loader,
                  }"
                  @click="item.supported && toggleLoader(item.loader)"
                >
                  <span class="loader-row__badge">{{ item.label.slice(0, 1) }}</span>
                  <div class="loader-row__info">
                    <strong>{{ item.label }}</strong>
                    <span>
                      {{ item.note }}
                      <template v-if="item.installed_id"> · {{ item.installed_id }}</template>
                    </span>
                  </div>
                  <div class="loader-row__action">
                    <template v-if="item.installed">
                      <span class="tag tag--grass">已安装</span>
                      <IconButton
                        icon="play"
                        :label="`启动 ${item.installed_id}`"
                        variant="primary"
                        :size="32"
                        :disabled="!!store.installingId"
                        @click.stop="launchLoader(item.installed_id)"
                      />
                    </template>
                    <IconButton
                      v-if="item.supported"
                      icon="chevron-down"
                      :label="expandedLoader === item.loader ? '收起版本列表' : '选择版本'"
                      variant="soft"
                      :size="32"
                      :class="{ 'loader-chevron--open': expandedLoader === item.loader }"
                      @click.stop="toggleLoader(item.loader)"
                    />
                    <span v-else class="muted small loader-row__unsupported">暂未支持</span>
                  </div>
                </div>

                <div v-if="expandedLoader === item.loader" class="loader-versions">
                  <div v-if="versionLoading === item.loader" class="loader-versions__state">
                    <span class="skeleton" />
                    <span class="muted small">正在获取 {{ item.label }} 版本…</span>
                  </div>
                  <div
                    v-else-if="versionError"
                    class="loader-versions__state loader-versions__state--error"
                  >
                    {{ versionError }}
                  </div>
                  <div
                    v-else-if="!(loaderVersionMap[item.loader] ?? []).length"
                    class="loader-versions__state muted small"
                  >
                    没有找到适配当前版本的 {{ item.label }}。
                  </div>
                  <div v-else class="loader-version-list">
                    <div
                      v-for="version in loaderVersionMap[item.loader]"
                      :key="version.version"
                      class="loader-version"
                    >
                      <span class="loader-version__name mono">{{ version.version }}</span>
                      <span v-if="version.recommended" class="tag tag--grass">推荐</span>
                      <span class="loader-version__install">
                        <IconButton
                          icon="download"
                          :label="`安装 ${item.label} ${version.version}`"
                          variant="soft"
                          :size="28"
                          :disabled="!!store.installingId"
                          @click.stop="installLoaderFor(item.loader, version.version)"
                        />
                      </span>
                    </div>
                  </div>
                </div>
              </template>
            </div>
          </section>
        </template>

        <template v-else>
          <section class="card catalog reveal" style="--i: 1">
            <div class="toolbar">
              <div class="tabs">
                <button
                  v-for="tab in tabs"
                  :key="tab.key"
                  class="tab"
                  :class="{ 'tab--active': filter === tab.key }"
                  @click="filter = tab.key; limit = 240"
                >
                  {{ tab.label }}
                </button>
              </div>
              <div class="toolbar__search">
                <WoodInput v-model="query" placeholder="搜索版本号，例如 1.20 / 26.3" />
              </div>
            </div>

            <div v-if="store.loadingVersions" class="skeleton-list">
              <div v-for="n in 9" :key="n" class="skeleton-row">
                <span class="skeleton skeleton-row__title" />
                <span class="skeleton skeleton-row__meta" />
                <span class="skeleton skeleton-row__btn" />
              </div>
            </div>
            <div v-else-if="!filtered.length" class="list-empty">没有匹配的版本</div>
            <div v-else class="version-list">
              <button
                v-for="(item, index) in filtered"
                :key="item.id"
                class="version-row"
                :style="{ '--i': index }"
                @click="openDetail(item.id)"
              >
                <span class="version-row__mark" />
                <span class="version-row__main">
                  <strong class="version-row__id">{{ item.id }}</strong>
                  <span
                    class="tag"
                    :class="{ 'tag--grass': item.kind === 'release' || APRIL_FOOLS_IDS.has(item.id) }"
                  >
                    {{ APRIL_FOOLS_IDS.has(item.id) ? "愚人节版" : kindLabel(item.kind) }}
                  </span>
                  <span v-if="item.installed" class="tag tag--grass">已安装</span>
                  <span class="version-row__date mono">{{ formatDate(item.release_time) }}</span>
                </span>
                <span class="version-row__action">
                  <span class="version-row__more">
                    详情
                    <AppIcon name="chevron-left" :size="14" class="version-row__chev" />
                  </span>
                </span>
              </button>
            </div>

            <div v-if="filtered.length < totalMatched" class="load-more">
              <WoodButton variant="ghost" size="sm" @click="limit += 200">
                <AppIcon name="chevron-down" :size="14" />
                加载更多（还有 {{ totalMatched - filtered.length }} 个）
              </WoodButton>
            </div>
          </section>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.download-layout {
  display: grid;
  grid-template-columns: 192px minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

/* 侧栏和联机页一致：整列当托盘，行距收紧、纯色块高亮，不做按键浮雕 */
.dl-side {
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  position: sticky;
  top: 0;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff);
}

.dl-side__item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  height: 34px;
  padding: 0 10px;
  border-radius: 0;
  color: var(--ink-soft);
  font-size: 13px;
  font-weight: 600;
  text-align: left;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    box-shadow var(--dur-1) ease;
}

.dl-side__item .icon {
  color: #b3a189;
  transition: color var(--dur-1) ease;
}

.dl-side__item:hover {
  background: var(--hover);
  color: var(--walnut-deep);
}

.dl-side__item:hover .icon {
  color: var(--walnut);
}

.dl-side__item--active,
.dl-side__item--active:hover {
  background: var(--moss-soft);
  color: var(--moss);
  box-shadow: inset 3px 0 0 var(--moss);
}

.dl-side__item--active .icon,
.dl-side__item--active:hover .icon {
  color: var(--moss);
}

.dl-side__label {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.18em;
  color: var(--muted);
  padding: 12px 10px 4px;
  text-transform: uppercase;
}

.dl-main {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

/* 详情 */
.detail-head {
  display: flex;
  align-items: center;
  gap: 14px;
}

.detail-head__text {
  flex: 1;
  min-width: 0;
}

.detail-title {
  font-size: 42px;
  line-height: 1.05;
  color: var(--walnut-deep);
}

.detail-head__tags {
  flex-shrink: 0;
}

.vanilla-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
}

.loader-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.loader-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
}

.loader-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 11px 13px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 70%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 70%, #000);
  background: var(--panel);
  transition: border-color var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo),
    box-shadow var(--dur-2) ease;
  cursor: pointer;
}

.loader-row:hover {
  transform: translateY(-1px);
}

/* 展开时按进槽里，和下面的版本列表连成一块 */
.loader-row--open,
.loader-row--open:hover {
  background: var(--moss-soft);
  border-top-color: color-mix(in srgb, var(--moss) 52%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 52%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 34%, #fff);
  border-bottom-width: 1px;
  border-bottom-color: color-mix(in srgb, var(--moss) 34%, #fff);
  transform: none;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.1);
}

.loader-chevron--open {
  transform: rotate(180deg);
}

.loader-versions {
  margin-top: -3px;
  border: 1px solid var(--line-strong);
  border-top: 1px dashed var(--line);
  border-radius: 0;
  background: rgba(var(--glass-rgb), 0.32);
  padding: 8px;
  max-height: 260px;
  overflow-y: auto;
}

.loader-versions__state {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 8px;
  color: var(--muted);
  font-size: 12px;
}

.loader-versions__state .skeleton {
  width: 90px;
  height: 12px;
}

.loader-versions__state--error {
  color: var(--danger);
}

.loader-version-list {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.loader-version {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 9px;
  border-radius: 0;
  transition: background var(--dur-1) ease, box-shadow var(--dur-1) ease;
}

.loader-version:hover {
  background: var(--hover);
  box-shadow: inset 3px 0 0 var(--moss);
}

.loader-version__name {
  flex: 1;
  font-size: 12px;
  color: var(--ink);
  font-family: var(--font-mono);
}

.loader-version__install {
  display: inline-flex;
}

.loader-row:hover {
  border-color: var(--walnut-light);
  transform: translateY(-1px);
}

.loader-row--open:hover {
  transform: none;
}

.loader-row--installed {
  border-color: var(--moss);
}

.loader-row__badge {
  width: 34px;
  height: 34px;
  border-radius: 4px;
  display: grid;
  place-items: center;
  background: var(--moss-soft);
  color: var(--moss);
  font-family: var(--font-display);
  font-size: 17px;
  font-weight: 700;
  flex-shrink: 0;
}

.loader-row__info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.loader-row__info strong {
  font-size: 14px;
  color: var(--walnut-deep);
}

.loader-row__info span {
  font-size: 11.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.loader-row__action {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.loader-row__unsupported {
  font-size: 11.5px;
  color: var(--muted);
}

.loader-row--skeleton {
  pointer-events: none;
}

.loader-skeleton__badge {
  width: 34px;
  height: 34px;
  border-radius: 4px;
}

.loader-skeleton__text {
  flex: 1;
  height: 14px;
}

.loader-skeleton__btn {
  width: 32px;
  height: 32px;
  border-radius: 4px;
}

/* 列表（沿用原有样式） */
.catalog {
  overflow: hidden;
}

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 18px;
  border-bottom: 1px solid var(--line);
  background: var(--panel);
}

.tabs {
  display: flex;
  gap: 3px;
  padding: 3px;
  background: var(--paper-2);
  border: 1px solid var(--line);
  border-radius: 2px;
}

/* 分段按键：未选中是平面，悬停浮起带浮雕边，选中则压进面板 */
.tab {
  --face: var(--panel-2);
  height: 29px;
  padding: 0 16px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  color: var(--ink-soft);
  font-size: 12.5px;
  font-weight: 700;
  transition: background 150ms ease, color 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
}

.tab:not(.tab--active):hover {
  color: var(--walnut-deep);
  background: var(--hover);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 42%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 42%, #000);
}

.tab:active {
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 42%, #000);
  border-left-color: color-mix(in srgb, var(--face) 42%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.18);
}

.tab--active {
  background: var(--panel-2);
  color: var(--walnut-deep);
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 42%, #000);
  border-left-color: color-mix(in srgb, var(--face) 42%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.18);
}

.toolbar__search {
  width: 300px;
}

.version-list {
  max-height: 560px;
  overflow-y: auto;
  padding: 7px 9px;
}

.version-row {
  position: relative;
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 11px 14px 11px 18px;
  border-radius: 0;
  text-align: left;
  color: var(--ink);
  transition: background var(--dur-2) ease;
  animation: rise-in 460ms var(--ease-out-expo) both;
  animation-delay: calc(var(--i, 0) * 18ms);
}

.version-row:hover {
  background: var(--hover);
}

.version-row__mark {
  position: absolute;
  left: 6px;
  top: 50%;
  width: 5px;
  height: 5px;
  border-radius: 0;
  background: var(--line-strong);
  transform: translateY(-50%);
  transition: background var(--dur-2) ease, transform var(--dur-2) var(--ease-out-expo);
}

.version-row:hover .version-row__mark {
  background: var(--moss);
  transform: translateY(-50%) scale(1.4);
}

.version-row__main {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.version-row__id {
  font-family: var(--font-display);
  font-size: 16px;
  color: var(--ink);
  min-width: 128px;
  letter-spacing: 0.01em;
}

.version-row__date {
  font-size: 10.5px;
  color: var(--muted);
  letter-spacing: 0.06em;
}

.version-row__action {
  flex-shrink: 0;
}

.version-row__more {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11.5px;
  color: var(--muted);
  transition: color var(--dur-2) ease;
}

.version-row:hover .version-row__more {
  color: var(--moss);
}

.version-row__chev {
  transform: rotate(180deg);
}

.skeleton-list {
  padding: 8px 12px;
}

.skeleton-row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 12px 8px;
}

.skeleton-row__title {
  width: 140px;
  height: 16px;
}

.skeleton-row__meta {
  width: 220px;
  height: 12px;
}

.skeleton-row__btn {
  margin-left: auto;
  width: 70px;
  height: 30px;
}

.list-empty {
  padding: 46px;
  text-align: center;
  color: var(--muted);
}

.load-more {
  display: flex;
  justify-content: center;
  padding: 13px;
  border-top: 1px dashed var(--line);
}

.download-progress {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

@media (max-width: 1080px) {
  .download-layout {
    grid-template-columns: 156px minmax(0, 1fr);
  }

  .toolbar__search {
    width: 220px;
  }
}
</style>
