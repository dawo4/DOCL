use crate::config::Config;
use anyhow::{anyhow, bail, Context, Result};
use quartz_nbt::io::{read_nbt, write_nbt, Flavor};
use quartz_nbt::{NbtCompound, NbtTag};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldRule {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldSettingsView {
    pub name: String,
    /// survival / creative / adventure / spectator
    pub game_mode: String,
    /// peaceful / easy / normal / hard
    pub difficulty: Option<String>,
    pub difficulty_locked: bool,
    pub hardcore: bool,
    pub allow_commands: Option<bool>,
    pub seed: Option<String>,
    pub version_name: Option<String>,
    pub game_rules: Vec<WorldRule>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WorldSettingsPatch {
    pub name: Option<String>,
    pub game_mode: Option<String>,
    pub difficulty: Option<String>,
    pub allow_commands: Option<bool>,
    /// 十进制 i64
    pub seed: Option<String>,
    pub game_rules: Option<Vec<WorldRule>>,
}

enum RuleSource {
    Level,
    File(PathBuf),
}

enum SeedSource {
    WorldGen,
    RandomSeed,
    File(PathBuf),
    None,
}

#[derive(Clone, Copy)]
enum DifficultyKind {
    Legacy,
    Settings,
}

struct Sources {
    rules: RuleSource,
    seed: SeedSource,
    difficulty: DifficultyKind,
    player: Option<PlayerTarget>,
}

enum PlayerTarget {
    LevelPlayer,
    File(PathBuf),
}

fn savedata_path(world: &Path, file: &str) -> PathBuf {
    world.join("data").join("minecraft").join(file)
}

fn read_root(path: &Path) -> Result<(NbtCompound, String)> {
    let raw =
        std::fs::read(path).with_context(|| format!("无法读取存档文件：{}", path.display()))?;
    let (root, name) = read_nbt(&mut Cursor::new(raw), Flavor::GzCompressed)
        .with_context(|| format!("{} 不是有效的存档数据", path.display()))?;
    Ok((root, name))
}

/// 写入前按原版习惯保留 `<文件名>_old`，再用临时文件原子替换。
fn write_root(path: &Path, root: &NbtCompound, root_name: &str) -> Result<()> {
    let original = std::fs::read(path)?;
    let mut backup = path.as_os_str().to_owned();
    backup.push("_old");
    std::fs::write(PathBuf::from(backup), original)?;

    let mut out = Vec::new();
    write_nbt(&mut out, Some(root_name), root, Flavor::GzCompressed)
        .context("生成存档数据失败")?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, out)?;
    std::fs::rename(&temp, path)
        .with_context(|| format!("写入存档失败：{}", path.display()))?;
    Ok(())
}

fn read_savedata(path: &Path) -> Result<NbtCompound> {
    let (root, _) = read_root(path)?;
    root.get::<_, &NbtCompound>("data")
        .cloned()
        .with_context(|| format!("{} 缺少 data 节点", path.display()))
}

fn modify_savedata(
    path: &Path,
    mutate: impl FnOnce(&mut NbtCompound) -> Result<()>,
) -> Result<()> {
    let (mut root, name) = read_root(path)?;
    {
        let data = root
            .get_mut::<_, &mut NbtCompound>("data")
            .with_context(|| format!("{} 缺少 data 节点", path.display()))?;
        mutate(data)?;
    }
    write_root(path, &root, &name)
}

fn numeric_nbt(tag: &NbtTag) -> Option<i64> {
    match tag {
        NbtTag::Byte(value) => Some(i64::from(*value)),
        NbtTag::Short(value) => Some(i64::from(*value)),
        NbtTag::Int(value) => Some(i64::from(*value)),
        NbtTag::Long(value) => Some(*value),
        _ => None,
    }
}

fn level_seed(data: &NbtCompound) -> Option<i64> {
    if let Ok(settings) = data.get::<_, &NbtCompound>("WorldGenSettings") {
        if let Ok(tag) = settings.get::<_, &NbtTag>("seed") {
            if let Some(seed) = numeric_nbt(tag) {
                return Some(seed);
            }
        }
    }
    data.get::<_, &NbtTag>("RandomSeed")
        .ok()
        .and_then(numeric_nbt)
}

fn uuid_from_parts(parts: &[i32]) -> Option<String> {
    if parts.len() != 4 {
        return None;
    }
    let hex: String = parts
        .iter()
        .map(|part| format!("{:08x}", *part as u32))
        .collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn probe(world: &Path, data: &NbtCompound) -> Sources {
    let rules = if data.get::<_, &NbtCompound>("GameRules").is_ok() {
        RuleSource::Level
    } else {
        let path = savedata_path(world, "game_rules.dat");
        if path.is_file() {
            RuleSource::File(path)
        } else {
            RuleSource::Level
        }
    };

    let seed = if data
        .get::<_, &NbtCompound>("WorldGenSettings")
        .ok()
        .and_then(|settings| settings.get::<_, &NbtTag>("seed").ok())
        .and_then(numeric_nbt)
        .is_some()
    {
        SeedSource::WorldGen
    } else if data
        .get::<_, &NbtTag>("RandomSeed")
        .ok()
        .and_then(numeric_nbt)
        .is_some()
    {
        SeedSource::RandomSeed
    } else {
        let path = savedata_path(world, "world_gen_settings.dat");
        if path.is_file() {
            SeedSource::File(path)
        } else {
            SeedSource::None
        }
    };

    let difficulty = if data.get::<_, i8>("Difficulty").is_ok() {
        DifficultyKind::Legacy
    } else if data.get::<_, &NbtCompound>("difficulty_settings").is_ok() {
        DifficultyKind::Settings
    } else {
        DifficultyKind::Legacy
    };

    let player = if data.get::<_, &NbtCompound>("Player").is_ok() {
        Some(PlayerTarget::LevelPlayer)
    } else {
        match data.get::<_, &NbtTag>("singleplayer_uuid") {
            Ok(NbtTag::IntArray(parts)) => uuid_from_parts(parts).map(|uuid| {
                PlayerTarget::File(
                    world
                        .join("players")
                        .join("data")
                        .join(format!("{uuid}.dat")),
                )
            }),
            _ => None,
        }
    };

    Sources {
        rules,
        seed,
        difficulty,
        player,
    }
}

fn difficulty_name(byte: i8) -> String {
    match byte {
        0 => "peaceful",
        1 => "easy",
        2 => "normal",
        _ => "hard",
    }
    .to_string()
}

fn difficulty_byte(value: &str) -> Result<i8> {
    Ok(match value {
        "peaceful" => 0,
        "easy" => 1,
        "normal" => 2,
        "hard" => 3,
        _ => bail!("未知的难度：{value}"),
    })
}

fn mode_name(game_type: i32) -> &'static str {
    match game_type {
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "survival",
    }
}

fn mode_int(value: &str) -> Result<i32> {
    Ok(match value {
        "survival" => 0,
        "creative" => 1,
        "adventure" => 2,
        "spectator" => 3,
        _ => bail!("未知的游戏模式：{value}"),
    })
}

fn rule_value(tag: &NbtTag) -> Option<String> {
    match tag {
        NbtTag::String(value) => Some(value.clone()),
        NbtTag::Byte(0) => Some("false".into()),
        NbtTag::Byte(1) => Some("true".into()),
        NbtTag::Byte(value) => Some(value.to_string()),
        NbtTag::Short(value) => Some(value.to_string()),
        NbtTag::Int(value) => Some(value.to_string()),
        NbtTag::Long(value) => Some(value.to_string()),
        _ => None,
    }
}

fn collect_rules(compound: &NbtCompound) -> Vec<WorldRule> {
    let mut rules: Vec<WorldRule> = compound
        .inner()
        .iter()
        .filter_map(|(key, tag)| {
            rule_value(tag).map(|value| WorldRule {
                key: key.clone(),
                value,
            })
        })
        .collect();
    rules.sort_by(|a, b| a.key.to_ascii_lowercase().cmp(&b.key.to_ascii_lowercase()));
    rules
}

fn apply_rules(target: &mut NbtCompound, rules: &[WorldRule]) -> Result<()> {
    for rule in rules {
        let Ok(existing) = target.get::<_, &NbtTag>(&rule.key) else {
            continue;
        };
        let value = rule.value.trim();
        let next = match existing {
            NbtTag::String(_) => NbtTag::String(value.to_string()),
            NbtTag::Byte(_) => match value {
                "true" => NbtTag::Byte(1),
                "false" => NbtTag::Byte(0),
                other => NbtTag::Byte(other.parse::<i8>().map_err(|_| {
                    anyhow!("游戏规则 {} 需要布尔值或数字", rule.key)
                })?),
            },
            NbtTag::Short(_) => NbtTag::Short(value.parse().map_err(|_| {
                anyhow!("游戏规则 {} 需要数字", rule.key)
            })?),
            NbtTag::Int(_) => NbtTag::Int(value.parse().map_err(|_| {
                anyhow!("游戏规则 {} 需要数字", rule.key)
            })?),
            NbtTag::Long(_) => NbtTag::Long(value.parse().map_err(|_| {
                anyhow!("游戏规则 {} 需要数字", rule.key)
            })?),
            _ => continue,
        };
        target.insert(rule.key.clone(), next);
    }
    Ok(())
}

/// 新版世界把种子写在多层嵌套里，所有名为 seed 的数字标签一起改。
fn set_seed_tags(compound: &mut NbtCompound, seed: i64) {
    for (name, tag) in compound.inner_mut() {
        if name.eq_ignore_ascii_case("seed") && numeric_nbt(tag).is_some() {
            *tag = NbtTag::Long(seed);
            continue;
        }
        match tag {
            NbtTag::Compound(child) => set_seed_tags(child, seed),
            NbtTag::List(list) => {
                for item in list.iter_mut() {
                    set_seed_in_tag(item, seed);
                }
            }
            _ => {}
        }
    }
}

fn set_seed_in_tag(tag: &mut NbtTag, seed: i64) {
    match tag {
        NbtTag::Compound(child) => set_seed_tags(child, seed),
        NbtTag::List(list) => {
            for item in list.iter_mut() {
                set_seed_in_tag(item, seed);
            }
        }
        _ => {}
    }
}

fn update_player_game_type(path: &Path, game_type: i32) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let (mut root, name) = read_root(path)?;
    root.insert("playerGameType", NbtTag::Int(game_type));
    write_root(path, &root, &name)
}

pub fn read_settings(cfg: &Config, version_id: &str, folder: &str) -> Result<WorldSettingsView> {
    let dir = crate::saves::world_dir(cfg, version_id, folder)?;
    let level = dir.join("level.dat");
    if !level.is_file() {
        bail!("存档缺少 level.dat");
    }
    let (root, _) = read_root(&level)?;
    let data = root
        .get::<_, &NbtCompound>("Data")
        .context("level.dat 缺少 Data 节点")?;
    let sources = probe(&dir, data);

    let rules = match &sources.rules {
        RuleSource::Level => data
            .get::<_, &NbtCompound>("GameRules")
            .map(collect_rules)
            .unwrap_or_default(),
        RuleSource::File(path) => read_savedata(path)
            .map(|compound| collect_rules(&compound))
            .unwrap_or_default(),
    };

    let seed = match &sources.seed {
        SeedSource::WorldGen | SeedSource::RandomSeed => level_seed(data),
        SeedSource::File(path) => read_savedata(path)
            .ok()
            .and_then(|compound| {
                compound
                    .get::<_, &NbtTag>("seed")
                    .ok()
                    .and_then(numeric_nbt)
            }),
        SeedSource::None => None,
    };

    let (difficulty, difficulty_locked, hardcore) = match sources.difficulty {
        DifficultyKind::Legacy => (
            data.get::<_, i8>("Difficulty")
                .ok()
                .map(difficulty_name),
            data.get::<_, i8>("DifficultyLocked").unwrap_or(0) != 0,
            data.get::<_, i8>("hardcore").unwrap_or(0) != 0,
        ),
        DifficultyKind::Settings => {
            let settings = data.get::<_, &NbtCompound>("difficulty_settings").ok();
            (
                settings
                    .and_then(|value| value.get::<_, &str>("difficulty").ok())
                    .map(str::to_string),
                settings
                    .map(|value| value.get::<_, i8>("locked").unwrap_or(0) != 0)
                    .unwrap_or(false),
                settings
                    .map(|value| value.get::<_, i8>("hardcore").unwrap_or(0) != 0)
                    .unwrap_or(false),
            )
        }
    };

    Ok(WorldSettingsView {
        name: data
            .get::<_, &str>("LevelName")
            .unwrap_or_default()
            .to_string(),
        game_mode: mode_name(data.get::<_, i32>("GameType").unwrap_or(0)).to_string(),
        difficulty,
        difficulty_locked,
        hardcore,
        allow_commands: data
            .get::<_, i8>("allowCommands")
            .ok()
            .map(|value| value != 0),
        seed: seed.map(|value| value.to_string()),
        version_name: data
            .get::<_, &NbtCompound>("Version")
            .ok()
            .and_then(|version| version.get::<_, &str>("Name").ok())
            .map(str::to_string),
        game_rules: rules,
    })
}

pub fn save_settings(
    cfg: &Config,
    version_id: &str,
    folder: &str,
    patch: WorldSettingsPatch,
) -> Result<WorldSettingsView> {
    let dir = crate::saves::world_dir(cfg, version_id, folder)?;
    let level = dir.join("level.dat");
    if !level.is_file() {
        bail!("存档缺少 level.dat");
    }
    let (mut root, root_name) = read_root(&level)?;
    let sources = {
        let data = root
            .get::<_, &NbtCompound>("Data")
            .context("level.dat 缺少 Data 节点")?;
        probe(&dir, data)
    };

    let mut level_dirty = false;
    let mut seed_file: Option<(PathBuf, i64)> = None;
    let mut rules_file: Option<(PathBuf, Vec<WorldRule>)> = None;

    {
        let data = root
            .get_mut::<_, &mut NbtCompound>("Data")
            .context("level.dat 缺少 Data 节点")?;

        if let Some(name) = patch.name.as_deref() {
            let name = name.trim();
            if !name.is_empty() {
                data.insert("LevelName", NbtTag::String(name.to_string()));
                level_dirty = true;
            }
        }

        if let Some(mode) = patch.game_mode.as_deref() {
            let game_type = mode_int(mode)?;
            data.insert("GameType", NbtTag::Int(game_type));
            level_dirty = true;
            match &sources.player {
                Some(PlayerTarget::LevelPlayer) => {
                    if let Ok(player) = data.get_mut::<_, &mut NbtCompound>("Player") {
                        player.insert("playerGameType", NbtTag::Int(game_type));
                    }
                }
                Some(PlayerTarget::File(path)) => {
                    update_player_game_type(path, game_type)?;
                }
                None => {}
            }
        }

        if let Some(difficulty) = patch.difficulty.as_deref() {
            match sources.difficulty {
                DifficultyKind::Legacy => {
                    data.insert("Difficulty", NbtTag::Byte(difficulty_byte(difficulty)?));
                    level_dirty = true;
                }
                DifficultyKind::Settings => {
                    if let Ok(settings) =
                        data.get_mut::<_, &mut NbtCompound>("difficulty_settings")
                    {
                        settings.insert(
                            "difficulty",
                            NbtTag::String(difficulty.to_string()),
                        );
                        level_dirty = true;
                    }
                }
            }
        }

        if let Some(allow) = patch.allow_commands {
            data.insert("allowCommands", NbtTag::Byte(allow as i8));
            level_dirty = true;
        }

        if let Some(seed) = patch.seed.as_deref() {
            let value = seed
                .trim()
                .parse::<i64>()
                .map_err(|_| anyhow!("种子必须是整数"))?;
            match &sources.seed {
                SeedSource::WorldGen => {
                    if let Ok(settings) =
                        data.get_mut::<_, &mut NbtCompound>("WorldGenSettings")
                    {
                        set_seed_tags(settings, value);
                        level_dirty = true;
                    }
                }
                SeedSource::RandomSeed => {
                    data.insert("RandomSeed", NbtTag::Long(value));
                    level_dirty = true;
                }
                SeedSource::File(path) => seed_file = Some((path.clone(), value)),
                SeedSource::None => bail!("这个存档没有可编辑的种子字段"),
            }
        }

        if let Some(rules) = patch.game_rules.as_deref() {
            match &sources.rules {
                RuleSource::Level => {
                    if let Ok(compound) =
                        data.get_mut::<_, &mut NbtCompound>("GameRules")
                    {
                        apply_rules(compound, rules)?;
                        level_dirty = true;
                    }
                }
                RuleSource::File(path) => {
                    rules_file = Some((path.clone(), rules.to_vec()));
                }
            }
        }
    }

    if level_dirty {
        write_root(&level, &root, &root_name)?;
    }
    if let Some((path, seed)) = seed_file {
        modify_savedata(&path, |data| {
            set_seed_tags(data, seed);
            Ok(())
        })?;
    }
    if let Some((path, rules)) = rules_file {
        modify_savedata(&path, |data| apply_rules(data, &rules))?;
    }

    read_settings(cfg, version_id, folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_world(root: &Path) -> PathBuf {
        let world = root.join("saves").join("test");
        std::fs::create_dir_all(&world).unwrap();
        let mut data = NbtCompound::new();
        data.insert("LevelName", NbtTag::String("测试世界".into()));
        data.insert("GameType", NbtTag::Int(0));
        data.insert("Difficulty", NbtTag::Byte(2));
        data.insert("allowCommands", NbtTag::Byte(0));
        data.insert("RandomSeed", NbtTag::Long(12345));
        data.insert("hardcore", NbtTag::Byte(0));
        let mut rules = NbtCompound::new();
        rules.insert("keepInventory", NbtTag::String("false".into()));
        rules.insert("randomTickSpeed", NbtTag::String("3".into()));
        data.insert("GameRules", NbtTag::Compound(rules));
        let mut version = NbtCompound::new();
        version.insert("Name", NbtTag::String("1.21.1".into()));
        data.insert("Version", NbtTag::Compound(version));
        let mut root_tag = NbtCompound::new();
        root_tag.insert("Data", NbtTag::Compound(data));
        let path = world.join("level.dat");
        let mut out = Vec::new();
        write_nbt(&mut out, Some(""), &root_tag, Flavor::GzCompressed).unwrap();
        std::fs::write(&path, out).unwrap();
        world
    }

    fn config_for(root: &Path) -> Config {
        let mut cfg = Config::default();
        cfg.game_dir = root.to_string_lossy().to_string();
        cfg.version_isolation = false;
        cfg
    }

    #[test]
    fn reads_legacy_world_settings() {
        let root = std::env::temp_dir().join(format!("docl-world-{}", uuid::Uuid::new_v4()));
        create_world(&root);
        let cfg = config_for(&root);
        let view = read_settings(&cfg, "1.21.1", "test").unwrap();
        assert_eq!(view.name, "测试世界");
        assert_eq!(view.game_mode, "survival");
        assert_eq!(view.difficulty.as_deref(), Some("normal"));
        assert_eq!(view.seed.as_deref(), Some("12345"));
        assert_eq!(view.version_name.as_deref(), Some("1.21.1"));
        assert!(view
            .game_rules
            .iter()
            .any(|rule| rule.key == "keepInventory" && rule.value == "false"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn saves_legacy_fields_and_rules() {
        let root = std::env::temp_dir().join(format!("docl-world-{}", uuid::Uuid::new_v4()));
        create_world(&root);
        let cfg = config_for(&root);
        let patch = WorldSettingsPatch {
            name: Some("改名后".into()),
            game_mode: Some("creative".into()),
            difficulty: Some("hard".into()),
            allow_commands: Some(true),
            seed: Some("99".into()),
            game_rules: Some(vec![
                WorldRule {
                    key: "keepInventory".into(),
                    value: "true".into(),
                },
                WorldRule {
                    key: "randomTickSpeed".into(),
                    value: "0".into(),
                },
            ]),
        };
        let view = save_settings(&cfg, "1.21.1", "test", patch).unwrap();
        assert_eq!(view.name, "改名后");
        assert_eq!(view.game_mode, "creative");
        assert_eq!(view.difficulty.as_deref(), Some("hard"));
        assert_eq!(view.allow_commands, Some(true));
        assert_eq!(view.seed.as_deref(), Some("99"));
        assert!(view
            .game_rules
            .iter()
            .any(|rule| rule.key == "keepInventory" && rule.value == "true"));
        // 原文件留了 _old 备份
        assert!(root.join("saves/test/level.dat_old").is_file());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_non_integer_seed() {
        let root = std::env::temp_dir().join(format!("docl-world-{}", uuid::Uuid::new_v4()));
        create_world(&root);
        let cfg = config_for(&root);
        let patch = WorldSettingsPatch {
            seed: Some("abc".into()),
            ..Default::default()
        };
        assert!(save_settings(&cfg, "1.21.1", "test", patch).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    fn write_savedata(world: &Path, file: &str, data: NbtCompound) {
        let dir = world.join("data").join("minecraft");
        std::fs::create_dir_all(&dir).unwrap();
        let mut root = NbtCompound::new();
        root.insert("data", NbtTag::Compound(data));
        let mut out = Vec::new();
        write_nbt(&mut out, Some(""), &root, Flavor::GzCompressed).unwrap();
        std::fs::write(dir.join(file), out).unwrap();
    }

    /// 新版世界：规则与种子在 data/minecraft 下拆分存放，难度写在 difficulty_settings 里
    #[test]
    fn reads_and_saves_split_world_data() {
        let root = std::env::temp_dir().join(format!("docl-world-{}", uuid::Uuid::new_v4()));
        let world = create_world(&root);

        let mut rules = NbtCompound::new();
        rules.insert("keepInventory", NbtTag::Byte(1));
        rules.insert("randomTickSpeed", NbtTag::Int(3));
        write_savedata(&world, "game_rules.dat", rules);

        let mut gen = NbtCompound::new();
        gen.insert("seed", NbtTag::Long(777));
        write_savedata(&world, "world_gen_settings.dat", gen);

        // 去掉旧格式字段，改成 difficulty_settings
        let (mut level, name) = read_root(&world.join("level.dat")).unwrap();
        {
            let data = level.get_mut::<_, &mut NbtCompound>("Data").unwrap();
            data.inner_mut().remove("GameRules");
            data.inner_mut().remove("RandomSeed");
            data.inner_mut().remove("Difficulty");
            let mut settings = NbtCompound::new();
            settings.insert("difficulty", NbtTag::String("normal".into()));
            settings.insert("locked", NbtTag::Byte(0));
            settings.insert("hardcore", NbtTag::Byte(0));
            data.insert("difficulty_settings", NbtTag::Compound(settings));
        }
        write_root(&world.join("level.dat"), &level, &name).unwrap();

        let cfg = config_for(&root);
        let view = read_settings(&cfg, "1.21.1", "test").unwrap();
        assert_eq!(view.seed.as_deref(), Some("777"));
        assert_eq!(view.difficulty.as_deref(), Some("normal"));
        assert!(view
            .game_rules
            .iter()
            .any(|rule| rule.key == "keepInventory" && rule.value == "true"));

        let patch = WorldSettingsPatch {
            difficulty: Some("hard".into()),
            seed: Some("888".into()),
            game_rules: Some(vec![
                WorldRule {
                    key: "keepInventory".into(),
                    value: "false".into(),
                },
                WorldRule {
                    key: "randomTickSpeed".into(),
                    value: "5".into(),
                },
            ]),
            ..Default::default()
        };
        let view = save_settings(&cfg, "1.21.1", "test", patch).unwrap();
        assert_eq!(view.difficulty.as_deref(), Some("hard"));
        assert_eq!(view.seed.as_deref(), Some("888"));

        let rules = read_savedata(&savedata_path(&world, "game_rules.dat")).unwrap();
        assert!(matches!(
            rules.get::<_, &NbtTag>("keepInventory").unwrap(),
            NbtTag::Byte(0)
        ));
        assert!(matches!(
            rules.get::<_, &NbtTag>("randomTickSpeed").unwrap(),
            NbtTag::Int(5)
        ));
        let gen = read_savedata(&savedata_path(&world, "world_gen_settings.dat")).unwrap();
        assert_eq!(gen.get::<_, &NbtTag>("seed").ok().and_then(numeric_nbt), Some(888));

        let (level, _) = read_root(&world.join("level.dat")).unwrap();
        let data = level.get::<_, &NbtCompound>("Data").unwrap();
        let settings = data
            .get::<_, &NbtCompound>("difficulty_settings")
            .unwrap();
        assert_eq!(
            settings.get::<_, &str>("difficulty").unwrap(),
            "hard"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
