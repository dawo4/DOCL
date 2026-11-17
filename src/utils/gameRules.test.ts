import { describe, expect, it } from "vitest";
import {
  gameRuleAlias,
  gameRuleHint,
  gameRuleId,
  gameRuleLabel,
  gameRuleRaw,
} from "./gameRules";

describe("gameRuleId", () => {
  it("去掉命名空间并转成驼峰", () => {
    expect(gameRuleId("minecraft:keep_inventory")).toBe("keepInventory");
    expect(gameRuleId("keepInventory")).toBe("keepInventory");
  });
});

describe("gameRuleLabel", () => {
  it("命中内置中文名", () => {
    expect(gameRuleLabel("minecraft:keep_inventory")).toBe("保留物品栏");
  });

  it("新版规则名通过别名映射到旧名称", () => {
    expect(gameRuleLabel("command_blocks_work")).toBe("启用命令方块");
    expect(gameRuleLabel("minecraft:advance_time")).toBe("时间推进");
    expect(gameRuleLabel("entity_drops")).toBe("实体掉落物");
    expect(gameRuleLabel("fire_spread_radius_around_player")).toBe("火焰蔓延半径");
  });

  it("未知规则回退到原始键名", () => {
    expect(gameRuleLabel("minecraft:some_future_rule")).toBe("some_future_rule");
  });
});

describe("gameRuleHint", () => {
  it("给部分规则补充说明", () => {
    expect(gameRuleHint("keep_inventory")).toContain("背包");
  });

  it("没有说明时返回空串", () => {
    expect(gameRuleHint("minecraft:random_tick_speed")).toBe("");
  });
});

describe("gameRuleAlias", () => {
  it("返回语言文件里的旧键名", () => {
    expect(gameRuleAlias("command_blocks_work")).toBe("commandBlocksEnabled");
    expect(gameRuleAlias("max_block_modifications")).toBe(
      "commandModificationBlockLimit"
    );
  });

  it("不需要别名时原样返回", () => {
    expect(gameRuleAlias("minecraft:keep_inventory")).toBe("keepInventory");
  });
});

describe("gameRuleRaw", () => {
  it("去掉命名空间方便显示", () => {
    expect(gameRuleRaw("minecraft:pvp")).toBe("pvp");
    expect(gameRuleRaw("pvp")).toBe("pvp");
  });
});
