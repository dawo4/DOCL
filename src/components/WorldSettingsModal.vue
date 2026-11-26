<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import {
  api,
  type GameRuleLang,
  type WorldRule,
  type WorldSettings,
  type WorldSettingsPatch,
} from "../api";
import AppIcon from "./AppIcon.vue";
import WoodButton from "./WoodButton.vue";
import WoodInput from "./WoodInput.vue";
import WoodModal from "./WoodModal.vue";
import WoodSelect from "./WoodSelect.vue";
import WoodToggle from "./WoodToggle.vue";
import { store } from "../store";
import { gameRuleAlias, gameRuleHint, gameRuleId, gameRuleLabel, gameRuleRaw } from "../utils/gameRules";

const props = defineProps<{ versionId: string; folder: string }>();
const emit = defineEmits<{
  close: [];
  saved: [settings: WorldSettings];
}>();

const loading = ref(true);
const saving = ref(false);
const error = ref("");
const settings = ref<WorldSettings | null>(null);

const draft = reactive({
  name: "",
  game_mode: "survival",
  difficulty: "normal",
  allow_commands: false,
  seed: "",
  rules: [] as WorldRule[],
});

const modeOptions = [
  { value: "survival", label: "生存" },
  { value: "creative", label: "创造" },
  { value: "adventure", label: "冒险" },
  { value: "spectator", label: "旁观" },
];

const difficultyOptions = [
  { value: "peaceful", label: "和平" },
  { value: "easy", label: "简单" },
  { value: "normal", label: "普通" },
  { value: "hard", label: "困难" },
];

const difficultyLocked = computed(() => !!settings.value?.difficulty_locked);
const hardcore = computed(() => !!settings.value?.hardcore);
const seedEditable = computed(() => settings.value?.seed !== null);
const allowCommandsEditable = computed(
  () => settings.value?.allow_commands !== null
);

const ruleQuery = ref("");
/// 从客户端 jar 与资源包读到的官方中文；读不到时用内置表兜底
const ruleLang = ref<Record<string, GameRuleLang>>({});

/// 新规则名可能只有旧键名有翻译，两个都试一次
function translationOf(key: string) {
  return ruleLang.value[gameRuleId(key)] ?? ruleLang.value[gameRuleAlias(key)];
}

function labelOf(key: string) {
  return translationOf(key)?.name || gameRuleLabel(key);
}

function hintOf(key: string) {
  return translationOf(key)?.description || gameRuleHint(key);
}

/// 按中文名排序，搜索时中英文都能命中
const visibleRules = computed(() => {
  const query = ruleQuery.value.trim().toLowerCase();
  return [...draft.rules]
    .filter((rule) => {
      if (!query) return true;
      return (
        labelOf(rule.key).toLowerCase().includes(query) ||
        gameRuleRaw(rule.key).toLowerCase().includes(query)
      );
    })
    .sort((a, b) => labelOf(a.key).localeCompare(labelOf(b.key), "zh-CN"));
});

function isBooleanRule(rule: WorldRule) {
  return rule.value === "true" || rule.value === "false";
}

function ruleEnabled(rule: WorldRule) {
  return rule.value === "true";
}

function setRule(rule: WorldRule, value: boolean) {
  rule.value = value ? "true" : "false";
}

function applySettings(value: WorldSettings) {
  settings.value = value;
  draft.name = value.name;
  draft.game_mode = value.game_mode;
  draft.difficulty = value.difficulty ?? "normal";
  draft.allow_commands = value.allow_commands ?? false;
  draft.seed = value.seed ?? "";
  draft.rules = value.game_rules.map((rule) => ({ ...rule }));
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const [value, translations] = await Promise.all([
      api.instanceWorldSettings(props.versionId, props.folder),
      api.instanceGameRuleLang(props.versionId).catch(() => ({})),
    ]);
    ruleLang.value = translations ?? {};
    applySettings(value);
  } catch (err) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

async function save() {
  if (saving.value || !settings.value) return;
  if (store.runningVersionId === props.versionId) {
    error.value = "游戏正在运行，请先退出再保存世界设置";
    return;
  }
  saving.value = true;
  error.value = "";
  const patch: WorldSettingsPatch = {
    name: draft.name.trim(),
    game_mode: draft.game_mode,
    allow_commands: allowCommandsEditable.value
      ? draft.allow_commands
      : undefined,
    seed: seedEditable.value && draft.seed.trim() ? draft.seed.trim() : undefined,
    game_rules: draft.rules.map((rule) => ({ ...rule })),
  };
  if (!difficultyLocked.value) patch.difficulty = draft.difficulty;
  try {
    const updated = await api.instanceSaveWorldSettings(
      props.versionId,
      props.folder,
      patch
    );
    emit("saved", updated);
    emit("close");
  } catch (err) {
    error.value = String(err);
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <WoodModal title="世界设置" :width="540" :z-index="92" @close="emit('close')">
    <div v-if="loading" class="world-loading">
      <span class="skeleton" />
      <span class="muted small">正在读取 level.dat…</span>
    </div>

    <div v-else-if="!settings" class="world-error">
      <AppIcon name="alert" :size="16" />
      <span>{{ error || "读取世界设置失败" }}</span>
      <WoodButton variant="ghost" size="sm" @click="load">重试</WoodButton>
    </div>

    <div v-else class="world-form">
      <label class="world-row">
        <span class="world-row__label">世界名称</span>
        <WoodInput v-model="draft.name" :maxlength="64" />
      </label>

      <label class="world-row">
        <span class="world-row__label">游戏模式</span>
        <WoodSelect v-model="draft.game_mode" :options="modeOptions" />
      </label>

      <label class="world-row">
        <span class="world-row__label">
          难度
          <span v-if="hardcore" class="world-tag">硬核</span>
        </span>
        <WoodSelect
          v-model="draft.difficulty"
          :options="difficultyOptions"
          :disabled="difficultyLocked"
        />
      </label>
      <p v-if="difficultyLocked" class="world-hint">
        这个存档在游戏里锁定了难度，需要先在游戏内解锁。
      </p>

      <div v-if="allowCommandsEditable" class="world-row">
        <span class="world-row__label">允许作弊</span>
        <WoodToggle v-model="draft.allow_commands" />
      </div>

      <label class="world-row">
        <span class="world-row__label">世界种子</span>
        <WoodInput
          v-model="draft.seed"
          mono
          :disabled="!seedEditable"
          placeholder="十进制整数"
        />
      </label>
      <p v-if="seedEditable" class="world-hint">
        只影响之后新生成的区块，已经生成的地形不会改变。
      </p>
      <p v-else class="world-hint">这个存档没有可编辑的种子字段。</p>

      <section v-if="draft.rules.length" class="world-rules">
        <header class="world-rules__head">
          <strong>游戏规则</strong>
          <span class="muted small">
            {{ draft.rules.length }} 项 · 显示 {{ visibleRules.length }} 项
          </span>
          <WoodInput
            v-model="ruleQuery"
            class="world-rules__search"
            placeholder="搜索中文名或规则名"
          />
        </header>
        <div class="world-rules__list">
          <div v-for="rule in visibleRules" :key="rule.key" class="rule-row">
            <span
              class="rule-row__name"
              :title="hintOf(rule.key) || rule.key"
            >
              <span class="rule-row__zh">{{ labelOf(rule.key) }}</span>
              <span class="rule-row__key mono">{{ gameRuleRaw(rule.key) }}</span>
            </span>
            <WoodToggle
              v-if="isBooleanRule(rule)"
              :model-value="ruleEnabled(rule)"
              @update:model-value="setRule(rule, $event)"
            />
            <WoodInput v-else v-model="rule.value" class="rule-row__input" />
          </div>
          <p v-if="!visibleRules.length" class="world-hint">
            没有匹配的游戏规则。
          </p>
        </div>
      </section>

      <p v-if="error" class="world-error-text">{{ error }}</p>
      <p class="world-note">保存前会自动把 level.dat 备份为 level.dat_old。</p>
    </div>

    <template #footer>
      <WoodButton variant="ghost" size="sm" @click="emit('close')">取消</WoodButton>
      <WoodButton
        variant="primary"
        size="sm"
        :disabled="loading || saving || !settings"
        @click="save"
      >
        {{ saving ? "保存中…" : "保存设置" }}
      </WoodButton>
    </template>
  </WoodModal>
</template>

<style scoped>
.world-loading,
.world-error {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 18px 4px;
  color: var(--muted);
}

.world-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.world-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
}

.world-row__label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-shrink: 0;
  font-size: var(--text-base);
  font-weight: 700;
  color: var(--ink-soft);
}

.world-row :deep(.input),
.world-row :deep(.select) {
  width: 230px;
  max-width: 100%;
}

.world-tag {
  padding: 1px 6px;
  border-radius: 2px;
  border: 1px solid var(--danger);
  color: var(--danger);
  font-size: var(--text-xs);
  font-weight: 700;
}

.world-hint,
.world-note {
  margin: -4px 0 0;
  font-size: var(--text-sm);
  color: var(--muted);
}

.world-note {
  margin-top: 4px;
}

.world-error-text {
  margin: 0;
  padding: 8px 10px;
  border-radius: 2px;
  background: var(--danger-soft);
  color: var(--danger);
  font-size: var(--text-base);
}

.world-rules {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin-top: 4px;
  padding: var(--space-4);
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
}

.world-rules__head {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px 10px;
}

.world-rules__head strong {
  font-size: var(--text-base);
  color: var(--walnut-deep);
}

.world-rules__search {
  width: 190px;
  margin-left: auto;
}

.world-rules__list {
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding-right: 4px;
}

.rule-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  min-height: 34px;
}

.rule-row__name {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.rule-row__zh {
  font-size: var(--text-base);
  font-weight: 700;
  color: var(--ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rule-row__key {
  font-size: var(--text-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rule-row__input {
  width: 150px;
  flex-shrink: 0;
}
</style>
