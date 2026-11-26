<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import AppIcon from "../components/AppIcon.vue";
import WoodButton from "../components/WoodButton.vue";
import { refreshFavorites, store, toggleFavorite } from "../store";
import type { Favorite } from "../api";

const emit = defineEmits<{ (e: "open", favorite: Favorite): void }>();

const KIND_LABELS: Record<string, string> = {
  mods: "Mod",
  resourcepacks: "资源包",
  shaders: "光影包",
  modpacks: "整合包",
  datapacks: "数据包",
  worlds: "地图",
};

const SOURCE_LABELS: Record<string, string> = {
  curseforge: "CurseForge",
  modrinth: "Modrinth",
};

const filter = ref("all");
const loading = ref(false);

/// 只列出收藏里实际存在的分类，避免出现空筛选项
const filterOptions = computed(() => {
  const kinds = new Set(store.favorites.map((item) => item.kind));
  return [
    { value: "all", label: "全部", count: store.favorites.length },
    ...[...kinds].map((kind) => ({
      value: kind,
      label: KIND_LABELS[kind] ?? kind,
      count: store.favorites.filter((item) => item.kind === kind).length,
    })),
  ];
});

const list = computed(() =>
  filter.value === "all"
    ? store.favorites
    : store.favorites.filter((item) => item.kind === filter.value)
);

onMounted(async () => {
  loading.value = true;
  await refreshFavorites();
  loading.value = false;
  // 收藏被清空后当前筛选可能已失效
  if (!filterOptions.value.some((option) => option.value === filter.value)) {
    filter.value = "all";
  }
});

function kindLabel(kind: string) {
  return KIND_LABELS[kind] ?? kind;
}

function sourceLabel(source: string) {
  return SOURCE_LABELS[source] ?? source;
}

function compactNumber(value: number) {
  if (value >= 100000000) return `${(value / 100000000).toFixed(1)} 亿`;
  if (value >= 10000) return `${(value / 10000).toFixed(1)} 万`;
  return String(value);
}

async function remove(favorite: Favorite) {
  await toggleFavorite(favorite.kind, favorite);
  if (!filterOptions.value.some((option) => option.value === filter.value)) {
    filter.value = "all";
  }
}
</script>

<template>
  <div class="fav">
    <section class="card card-pad fav-head">
      <div>
        <div class="section-title">我的收藏</div>
        <p class="muted small">
          收藏的 Mod、整合包等资源都汇总在这里，点击可直接打开下载页；再次点击星标取消收藏。
        </p>
      </div>
      <span class="tag">{{ store.favorites.length }} 个</span>
    </section>

    <div v-if="filterOptions.length > 1" class="fav-filters">
      <button
        v-for="option in filterOptions"
        :key="option.value"
        class="fav-chip"
        :class="{ 'fav-chip--on': filter === option.value }"
        @click="filter = option.value"
      >
        {{ option.label }}
        <span class="fav-chip__count">{{ option.count }}</span>
      </button>
    </div>

    <div v-if="loading && !store.favorites.length" class="card card-pad">
      <div class="loading-rows">
        <span class="loading-rows__row" />
        <span class="loading-rows__row" />
        <span class="loading-rows__row" />
      </div>
    </div>

    <div v-else-if="!list.length" class="card card-pad fav-state">
      <AppIcon name="star" :size="30" />
      <div>
        <p class="fav-state__title">
          {{ store.favorites.length ? "这个分类下还没有收藏" : "还没有收藏任何资源" }}
        </p>
        <p class="muted small">
          在 Mod、整合包等页面点项目右侧的星标，就会收进这里。
        </p>
      </div>
    </div>

    <div v-else class="card fav-list">
      <div
        v-for="favorite in list"
        :key="`${favorite.source}-${favorite.id}`"
        class="fav-row"
        role="button"
        tabindex="0"
        @click="emit('open', favorite)"
        @keydown.enter.prevent="emit('open', favorite)"
      >
        <img
          v-if="favorite.logo_url"
          class="fav-row__logo"
          :src="favorite.logo_url"
          alt=""
          loading="lazy"
        />
        <span v-else class="fav-row__logo fav-row__logo--empty">
          {{ favorite.name.slice(0, 1) }}
        </span>

        <div class="fav-row__body">
          <div class="fav-row__title">
            <strong class="fav-row__name">{{ favorite.name }}</strong>
            <span class="tag">{{ sourceLabel(favorite.source) }}</span>
            <span class="chip">{{ kindLabel(favorite.kind) }}</span>
            <span
              v-for="category in favorite.categories.slice(0, 2)"
              :key="category"
              class="chip"
            >
              {{ category }}
            </span>
          </div>
          <p class="fav-row__summary">{{ favorite.summary || "暂无简介" }}</p>
        </div>

        <div class="fav-row__meta">
          <span class="fav-row__stat">
            <AppIcon name="user" :size="12" />
            <span>{{ favorite.author || "未知作者" }}</span>
          </span>
          <span class="fav-row__stat fav-row__stat--count">
            <AppIcon name="download" :size="12" />
            <span>{{ compactNumber(favorite.download_count) }}</span>
          </span>
        </div>

        <button
          class="fav-row__star"
          aria-label="取消收藏"
          @click.stop="remove(favorite)"
        >
          <AppIcon name="star" :size="16" />
        </button>
      </div>
    </div>

    <div v-if="store.favorites.length" class="fav-foot">
      <WoodButton variant="ghost" size="sm" @click="refreshFavorites">
        <AppIcon name="refresh" :size="14" />
        刷新收藏
      </WoodButton>
    </div>
  </div>
</template>

<style scoped>
.fav {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.fav-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-6);
}

.fav-filters {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
}

.fav-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: 5px 11px;
  font-size: var(--text-base);
  color: var(--ink-soft);
  background: var(--panel);
  border: 2px solid var(--frame);
  border-radius: 2px;
  transition: color var(--dur-1) ease, background var(--dur-1) ease;
}

.fav-chip:hover {
  color: var(--walnut-deep);
  background: var(--hover);
}

.fav-chip--on {
  color: var(--moss);
  background: var(--moss-soft);
  border-color: var(--moss);
}

.fav-chip__count {
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  opacity: 0.7;
}

.fav-state {
  display: flex;
  align-items: center;
  gap: 13px;
  color: var(--muted);
}

.fav-state__title {
  color: var(--ink);
  font-weight: 600;
}

.fav-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  border: 2px solid var(--frame);
  border-radius: 2px;
  background: var(--line);
  overflow: hidden;
}

.fav-row {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr) 210px 30px;
  align-items: center;
  gap: 14px;
  padding: 11px 16px;
  text-align: left;
  background: var(--panel);
  cursor: pointer;
  transition: background var(--dur-1) ease;
}

.fav-row:hover {
  background: rgba(var(--accent-rgb), 0.05);
}

.fav-row:focus-visible {
  outline: 2px solid rgba(94, 123, 73, 0.55);
  outline-offset: -2px;
}

.fav-row__logo {
  width: 48px;
  height: 48px;
  border-radius: 2px;
  object-fit: cover;
  background: var(--paper-2);
}

.fav-row__logo--empty {
  display: grid;
  place-items: center;
  font-family: var(--font-display);
  font-size: 20px;
  color: var(--moss);
  background: var(--moss-soft);
}

.fav-row__body {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.fav-row__title {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  flex-wrap: wrap;
}

.fav-row__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fav-row__summary {
  overflow: hidden;
  color: var(--muted);
  font-size: var(--text-base);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fav-row__meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 3px;
  min-width: 0;
  color: var(--ink-soft);
  font-size: var(--text-base);
}

.fav-row__stat {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 100%;
  overflow: hidden;
  white-space: nowrap;
}

.fav-row__stat--count {
  font-family: var(--font-mono);
  color: var(--muted);
}

.fav-row__star {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  color: var(--amber);
  border-radius: 2px;
  transition: background var(--dur-1) ease;
}

.fav-row__star:hover {
  background: rgba(var(--accent-rgb), 0.08);
}

.fav-row__star .icon {
  fill: currentColor;
}

.fav-foot {
  display: flex;
  justify-content: flex-end;
}
</style>
