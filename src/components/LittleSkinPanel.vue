<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  api,
  openExternal,
  type LittleSkinClosetItem,
  type LittleSkinClosetPage,
  type LittleSkinPlayer,
  type LittleSkinStatus,
} from "../api";
import { notify } from "../store";
import AppIcon from "./AppIcon.vue";
import WoodButton from "./WoodButton.vue";
import WoodInput from "./WoodInput.vue";
import WoodSelect from "./WoodSelect.vue";

const props = withDefaults(defineProps<{ keyword?: string }>(), { keyword: "" });

const status = ref<LittleSkinStatus | null>(null);
const loading = ref(true);
const error = ref("");
const clientId = ref("");
const clientSecret = ref("");
const saving = ref(false);
const authorizing = ref(false);
const closet = ref<LittleSkinClosetPage | null>(null);
const closetLoading = ref(false);
const players = ref<LittleSkinPlayer[]>([]);
const targetPid = ref("");
const applyingTid = ref(0);

const playerOptions = computed(() =>
  players.value.map((player) => ({ value: String(player.pid), label: player.name }))
);
const targetPlayer = computed(
  () => players.value.find((player) => String(player.pid) === targetPid.value) ?? null
);
const visibleItems = computed(() => {
  const list = closet.value?.items ?? [];
  const key = props.keyword.trim().toLowerCase();
  if (!key) return list;
  return list.filter((item) => item.name.toLowerCase().includes(key));
});

async function loadPlayers() {
  try {
    players.value = await api.littleskinPlayers();
    if (!targetPid.value && players.value.length) {
      targetPid.value = String(players.value[0].pid);
    }
  } catch (err) {
    error.value = String(err);
  }
}

let closetSeq = 0;

async function loadCloset(page: number) {
  const seq = ++closetSeq;
  closetLoading.value = true;
  try {
    const pageData = await api.littleskinCloset(page);
    // 授权后刷新与手动翻页可能并发，别让旧请求覆盖新页
    if (seq === closetSeq) closet.value = pageData;
  } catch (err) {
    if (seq === closetSeq) error.value = String(err);
  } finally {
    if (seq === closetSeq) closetLoading.value = false;
  }
}

async function loadAll() {
  await Promise.all([loadPlayers(), loadCloset(1)]);
}

async function loadStatus() {
  loading.value = true;
  error.value = "";
  try {
    status.value = await api.littleskinStatus();
    if (status.value.authorized) await loadAll();
  } catch (err) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

async function saveApp() {
  saving.value = true;
  error.value = "";
  try {
    status.value = await api.littleskinSetApp(clientId.value, clientSecret.value);
    clientSecret.value = "";
    notify("已保存 LittleSkin 应用凭据");
  } catch (err) {
    error.value = String(err);
  } finally {
    saving.value = false;
  }
}

async function authorize() {
  authorizing.value = true;
  error.value = "";
  try {
    status.value = await api.littleskinAuthorize();
    notify(`已连接 LittleSkin：${status.value.name || "账号"}`);
    await loadAll();
  } catch (err) {
    error.value = String(err);
  } finally {
    authorizing.value = false;
  }
}

async function logout() {
  error.value = "";
  try {
    await api.littleskinLogout();
    status.value = await api.littleskinStatus();
    closet.value = null;
    players.value = [];
    targetPid.value = "";
  } catch (err) {
    error.value = String(err);
  }
}

async function apply(item: LittleSkinClosetItem) {
  const player = targetPlayer.value;
  if (!player) {
    error.value = "请先选择一个角色";
    return;
  }
  applyingTid.value = item.tid;
  error.value = "";
  try {
    await api.littleskinApply(player.pid, item.tid);
    notify(`已把「${item.name}」应用到 ${player.name}`);
    await loadPlayers();
  } catch (err) {
    error.value = String(err);
  } finally {
    applyingTid.value = 0;
  }
}

function copyRedirect() {
  const value = status.value?.redirect_uri ?? "";
  navigator.clipboard
    ?.writeText(value)
    .then(() => notify("已复制回调地址"))
    .catch(() => notify("复制失败，请手动选中复制", "err"));
}

onMounted(loadStatus);
</script>

<template>
  <div class="ls">
    <div v-if="loading" class="hint">正在读取 LittleSkin 状态…</div>

    <template v-else-if="status">
      <section v-if="!status.configured" class="setup">
        <p class="desc">
          LittleSkin 是第三方皮肤站，不属于 DOCL。先在 LittleSkin 创建 OAuth 应用，回调地址填下面这个本地地址，保存后即可授权读取衣柜。
        </p>
        <div class="redirect">
          <span class="mono">{{ status.redirect_uri }}</span>
          <WoodButton variant="ghost" size="sm" @click="copyRedirect">复制</WoodButton>
          <WoodButton
            variant="ghost"
            size="sm"
            @click="openExternal('https://littleskin.cn/user/oauth/manage')"
          >
            <AppIcon name="external" :size="13" />
            打开管理页
          </WoodButton>
        </div>
        <div class="setup__row">
          <label class="field">
            <span class="field__label">Client ID</span>
            <WoodInput v-model="clientId" placeholder="应用详情里的 Client ID" mono />
          </label>
          <label class="field">
            <span class="field__label">Client Secret</span>
            <WoodInput
              v-model="clientSecret"
              type="password"
              placeholder="应用详情里的 Client Secret"
              mono
            />
          </label>
        </div>
        <div class="actions">
          <WoodButton variant="primary" size="sm" :disabled="saving" @click="saveApp">
            {{ saving ? "保存中…" : "保存凭据" }}
          </WoodButton>
        </div>
      </section>

      <section v-else-if="!status.authorized" class="setup">
        <p class="desc">
          LittleSkin 是第三方皮肤站，不属于 DOCL。点击授权后会在浏览器里打开 LittleSkin
          登录页，同意授权即可读取你的衣柜，并把选中的皮肤应用到站点角色。授权有效期约 3 天。
        </p>
        <div class="actions">
          <WoodButton variant="primary" size="sm" :disabled="authorizing" @click="authorize">
            <AppIcon name="external" :size="14" />
            {{ authorizing ? "等待浏览器完成授权…" : "在浏览器中授权" }}
          </WoodButton>
          <WoodButton variant="ghost" size="sm" :disabled="authorizing" @click="logout">
            重新填写应用凭据
          </WoodButton>
        </div>
        <p v-if="authorizing" class="muted small">已打开浏览器，请在网页里登录并同意授权（最长等待 5 分钟）。</p>
      </section>

      <template v-else>
        <div class="who">
          <span>
            已连接 <strong>{{ status.name || "LittleSkin 账号" }}</strong>
            <span v-if="status.uid" class="muted small">UID {{ status.uid }}</span>
          </span>
          <div class="who__right">
            <WoodSelect v-if="playerOptions.length" v-model="targetPid" :options="playerOptions" />
            <span v-else class="muted small">该账号还没有创建角色</span>
            <WoodButton variant="ghost" size="sm" @click="logout">断开</WoodButton>
          </div>
        </div>

        <div v-if="closetLoading" class="hint">正在读取衣柜…</div>
        <div v-else-if="closet && !closet.items.length" class="hint">
          衣柜是空的，可以先去 LittleSkin 收藏一些皮肤。
        </div>
        <div v-else-if="closet && !visibleItems.length" class="hint">
          没有匹配「{{ keyword }}」的皮肤。
        </div>
        <div v-else-if="closet" class="grid">
          <article v-for="item in visibleItems" :key="item.tid" class="cell">
            <div class="cell__preview">
              <img :src="item.preview" :alt="item.name" loading="lazy" />
              <span v-if="targetPlayer && targetPlayer.tid_skin === item.tid" class="cell__badge">
                使用中
              </span>
            </div>
            <div class="cell__name" :title="item.name">{{ item.name }}</div>
            <WoodButton
              variant="ghost"
              size="sm"
              :disabled="applyingTid === item.tid"
              @click="apply(item)"
            >
              {{ applyingTid === item.tid ? "应用中…" : "使用" }}
            </WoodButton>
          </article>
        </div>

        <div v-if="closet && closet.last_page > 1" class="pager">
          <WoodButton
            variant="ghost"
            size="sm"
            :disabled="closet.page <= 1 || closetLoading"
            @click="loadCloset(closet.page - 1)"
          >
            上一页
          </WoodButton>
          <span class="muted small">{{ closet.page }} / {{ closet.last_page }}</span>
          <WoodButton
            variant="ghost"
            size="sm"
            :disabled="closet.page >= closet.last_page || closetLoading"
            @click="loadCloset(closet.page + 1)"
          >
            下一页
          </WoodButton>
        </div>
      </template>
    </template>

    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>

<style scoped>
.ls {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.setup {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.desc {
  margin: 0;
  padding: 12px 14px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--ink-soft);
  font-size: 12.5px;
  line-height: 1.75;
}

.redirect {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px dashed var(--line-strong);
  border-radius: 2px;
  background: var(--panel);
  overflow: hidden;
}

.redirect .mono {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  color: var(--ink-soft);
}

.setup__row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field__label {
  font-size: 11px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--muted);
}

.actions {
  display: flex;
  align-items: center;
  gap: 10px;
}

.who {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--panel-2);
  font-size: 12.5px;
  color: var(--ink-soft);
}

.who strong {
  color: var(--ink);
}

.who__right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hint {
  padding: 26px 0;
  text-align: center;
  color: var(--muted);
  font-size: 12px;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(132px, 1fr));
  gap: 10px;
}

.cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 8px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--panel-2);
  transition: transform var(--dur-1) ease, border-color var(--dur-1) ease;
}

.cell:hover {
  transform: translateY(-1px);
  border-color: var(--line-strong);
}

.cell__preview {
  position: relative;
  width: 100%;
  aspect-ratio: 1;
  display: grid;
  place-items: center;
  background: color-mix(in srgb, var(--panel) 70%, transparent);
  border-radius: 2px;
  overflow: hidden;
}

.cell__preview img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}

.cell__badge {
  position: absolute;
  top: 4px;
  right: 4px;
  padding: 1px 6px;
  border-radius: 2px;
  background: var(--moss);
  color: #fffdf6;
  font-size: 10px;
}

.cell__name {
  width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: center;
  font-size: 11.5px;
  color: var(--ink-soft);
}

.pager {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  margin-top: 4px;
}

.error {
  margin: 0;
  padding: 9px 12px;
  border-radius: 2px;
  border: 1px solid color-mix(in srgb, #b65a3c 45%, transparent);
  background: color-mix(in srgb, #b65a3c 10%, transparent);
  color: #8f4126;
  font-size: 12px;
  line-height: 1.6;
}
</style>
