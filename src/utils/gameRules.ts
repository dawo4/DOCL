/// Minecraft 游戏规则的中文名。键统一用驼峰形式，
/// 这样 `keepInventory` 和 `minecraft:keep_inventory` 都能命中同一条。
const RULE_NAMES: Record<string, { name: string; hint?: string }> = {
  advanceTime: { name: "时间推进", hint: "是否让世界时间自然流逝" },
  advanceWeather: { name: "天气更替", hint: "是否让天气自然变化" },
  allowEnteringNetherUsingPortals: {
    name: "允许进入下界",
    hint: "玩家能否通过下界传送门进入下界",
  },
  announceAdvancements: { name: "进度通告", hint: "获得进度时是否在聊天栏播报" },
  blockDrops: { name: "方块掉落物", hint: "破坏方块时是否产生掉落物" },
  blockExplosionDropDecay: { name: "方块爆炸掉落衰减" },
  commandBlockOutput: { name: "命令方块输出", hint: "命令方块执行时是否向聊天栏输出" },
  commandBlocksEnabled: { name: "启用命令方块" },
  commandModificationBlockLimit: {
    name: "命令修改方块数量限制",
    hint: "单条命令（如 fill 和 clone）最多能更改的方块数量",
  },
  doDaylightCycle: { name: "昼夜更替" },
  doEntityDrops: { name: "实体掉落物" },
  doFireTick: { name: "火焰蔓延" },
  doImmediateRespawn: { name: "立即重生" },
  doInsomnia: { name: "幻翼生成" },
  doLimitedCrafting: { name: "限制合成" },
  doMobLoot: { name: "生物掉落物" },
  doMobSpawning: { name: "生物生成" },
  doPatrolSpawning: { name: "巡逻队生成" },
  doTileDrops: { name: "方块掉落物" },
  doTraderSpawning: { name: "流浪商人生成" },
  doVinesSpread: { name: "藤蔓蔓延" },
  doWardenSpawning: { name: "监守者生成" },
  doWeatherCycle: { name: "天气更替" },
  drowningDamage: { name: "溺水伤害" },
  elytraMovementCheck: { name: "鞘翅移动检测" },
  enderPearlsVanishOnDeath: { name: "末影珍珠死亡消失" },
  fallDamage: { name: "摔落伤害" },
  fireDamage: { name: "火焰伤害" },
  fireSpreadRadiusAroundPlayer: {
    name: "火焰蔓延半径",
    hint: "玩家周围火可以蔓延的方块半径",
  },
  forgiveDeadPlayers: { name: "宽恕死亡玩家" },
  freezeDamage: { name: "冰冻伤害" },
  globalSoundEvents: { name: "全局声音事件" },
  keepInventory: { name: "保留物品栏", hint: "死亡后保留背包与经验" },
  lavaSourceConversion: { name: "岩浆源转换" },
  locatorBar: { name: "玩家定位栏", hint: "是否在屏幕上显示指示玩家方位的定位栏" },
  logAdminCommands: { name: "记录管理员命令" },
  maxCommandChainLength: { name: "命令链长度上限" },
  maxCommandForkCount: { name: "命令分叉上限" },
  maxEntityCramming: { name: "实体挤压上限" },
  mobExplosionDropDecay: { name: "生物爆炸掉落衰减" },
  mobGriefing: { name: "生物破坏" },
  naturalRegeneration: { name: "自然生命恢复" },
  playersSleepingPercentage: { name: "玩家睡眠比例" },
  playersNetherPortalCreativeDelay: {
    name: "创造模式传送门等待时间",
    hint: "创造模式下通过下界传送门需要等待的刻数",
  },
  playersNetherPortalDefaultDelay: {
    name: "生存模式传送门等待时间",
    hint: "非创造模式下通过下界传送门需要等待的刻数",
  },
  playerMovementCheck: { name: "玩家移动检测" },
  projectilesCanBreakBlocks: { name: "弹射物破坏方块" },
  pvp: { name: "玩家互相伤害", hint: "控制玩家之间能否互相伤害" },
  raids: { name: "袭击", hint: "是否允许袭击发生" },
  randomTickSpeed: { name: "随机刻速度" },
  reducedDebugInfo: { name: "简化调试信息" },
  sendCommandFeedback: { name: "命令反馈" },
  showDeathMessages: { name: "死亡消息" },
  snowAccumulationHeight: { name: "积雪高度" },
  spectatorsGenerateChunks: { name: "旁观者生成区块" },
  spawnChunkRadius: { name: "出生点区块半径" },
  spawnMonsters: { name: "生成怪物", hint: "怪物能否自然生成" },
  spawnerBlocksEnabled: { name: "启用刷怪笼" },
  spawnRadius: { name: "出生点保护半径" },
  tntExplodes: { name: "启用 TNT 爆炸" },
  tntExplosionDropDecay: { name: "TNT 爆炸掉落衰减" },
  universalAnger: { name: "全局愤怒" },
  waterSourceConversion: { name: "水源转换" },
};

function camelize(key: string) {
  const bare = key.replace(/^minecraft:/, "");
  if (!bare.includes("_")) return bare;
  return bare.replace(/_([a-z0-9])/g, (_, ch: string) => ch.toUpperCase());
}

/// 新版把不少规则改成了新的 snake_case 名字，语言文件里仍是旧名字，
/// 这里把新名字映射回旧名字，翻译表才能命中。
const RULE_ALIASES: Record<string, string> = {
  advanceTime: "doDaylightCycle",
  advanceWeather: "doWeatherCycle",
  blockDrops: "doTileDrops",
  commandBlocksWork: "commandBlocksEnabled",
  entityDrops: "doEntityDrops",
  immediateRespawn: "doImmediateRespawn",
  limitedCrafting: "doLimitedCrafting",
  maxBlockModifications: "commandModificationBlockLimit",
  maxCommandForks: "maxCommandForkCount",
  maxCommandSequenceLength: "maxCommandChainLength",
  maxSnowAccumulationHeight: "snowAccumulationHeight",
  mobDrops: "doMobLoot",
  naturalHealthRegeneration: "naturalRegeneration",
  respawnRadius: "spawnRadius",
  showAdvancementMessages: "announceAdvancements",
  spawnMobs: "doMobSpawning",
  spawnPatrols: "doPatrolSpawning",
  spawnPhantoms: "doInsomnia",
  spawnWanderingTraders: "doTraderSpawning",
  spawnWardens: "doWardenSpawning",
  spawnerBlocksWork: "spawnerBlocksEnabled",
  spreadVines: "doVinesSpread",
};

/// 供语言文件查表：去掉命名空间并统一成驼峰
export function gameRuleId(key: string) {
  return camelize(key);
}

/// 翻译表里实际使用的键名：新规则名映射回旧键名
export function gameRuleAlias(key: string) {
  const id = camelize(key);
  return RULE_ALIASES[id] ?? id;
}

function ruleEntry(key: string) {
  const id = camelize(key);
  return RULE_NAMES[id] ?? RULE_NAMES[RULE_ALIASES[id] ?? ""];
}

export function gameRuleLabel(key: string) {
  return ruleEntry(key)?.name ?? key.replace(/^minecraft:/, "");
}

export function gameRuleHint(key: string) {
  return ruleEntry(key)?.hint ?? "";
}

export function gameRuleRaw(key: string) {
  return key.replace(/^minecraft:/, "");
}
