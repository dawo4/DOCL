/// BMCLAPI 镜像地址重写。
/// 文档：https://bmclapidoc.bangbang93.com/

const HOST_MAPPINGS: &[(&str, &str)] = &[
    (
        "https://launchermeta.mojang.com/",
        "https://bmclapi2.bangbang93.com/",
    ),
    (
        "https://launcher.mojang.com/",
        "https://bmclapi2.bangbang93.com/",
    ),
    (
        "https://piston-meta.mojang.com/",
        "https://bmclapi2.bangbang93.com/",
    ),
    (
        "https://piston-data.mojang.com/",
        "https://bmclapi2.bangbang93.com/",
    ),
    (
        "https://libraries.minecraft.net/",
        "https://bmclapi2.bangbang93.com/maven/",
    ),
    // Forge 的规范路径带 /releases/，必须先于通用主机映射匹配，
    // 否则会被拼成 /maven/net/minecraftforge/releases/... 这个不存在的地址
    (
        "https://maven.minecraftforge.net/releases/",
        "https://bmclapi2.bangbang93.com/maven/",
    ),
    (
        "https://maven.minecraftforge.net/",
        "https://bmclapi2.bangbang93.com/maven/",
    ),
    // NeoForge 安装器里的库全部指向 maven.neoforged.net；BMCLAPI 有同路径镜像，
    // 不映射的话国内会直连境外源，安装原生步骤很容易超时后回退到退出码 1
    (
        "https://maven.neoforged.net/releases/",
        "https://bmclapi2.bangbang93.com/maven/",
    ),
    (
        "https://resources.download.minecraft.net/",
        "https://bmclapi2.bangbang93.com/assets/",
    ),
    (
        "https://meta.fabricmc.net/",
        "https://bmclapi2.bangbang93.com/fabric-meta/",
    ),
    (
        "https://maven.fabricmc.net/",
        "https://bmclapi2.bangbang93.com/maven/",
    ),
    (
        "https://meta.quiltmc.org/",
        "https://bmclapi2.bangbang93.com/quilt-meta/",
    ),
    (
        "https://maven.quiltmc.org/repository/release/",
        "https://bmclapi2.bangbang93.com/maven/",
    ),
    // CurseForge CDN：国内直连 forgecdn 常超时或 403，走 MCIM 镜像（路径规则一致）
    ("https://edge.forgecdn.net/", "https://mod.mcimirror.top/"),
    (
        "https://mediafilez.forgecdn.net/",
        "https://mod.mcimirror.top/",
    ),
    ("https://media.forgecdn.net/", "https://mod.mcimirror.top/"),
    // Modrinth CDN：国内多数网络直接不可达，MCIM 同样按路径代理
    ("https://cdn.modrinth.com/", "https://mod.mcimirror.top/"),
    // Modrinth 接口：搜索/项目/版本查询同样常被阻断，镜像路径带 /modrinth/v2
    (
        "https://api.modrinth.com/v2/",
        "https://mod.mcimirror.top/modrinth/v2/",
    ),
];

pub fn rewrite(url: &str) -> String {
    // BMCLAPI 不代理 Java 运行时索引：会 302 到一个连不上的地址，实测挂满 25 秒。
    // 官方源本身 0.36 秒就能返回，保持原样。
    if url.contains("/v1/products/java-runtime/") {
        return url.to_string();
    }
    for (from, to) in HOST_MAPPINGS {
        if let Some(rest) = url.strip_prefix(from) {
            return format!("{to}{rest}");
        }
    }
    // 未收录的主机不做改写，避免把外部 API 拼成镜像子路径
    url.to_string()
}

/// 该地址是否应该镜像优先。
/// 只有 CurseForge 的 forgecdn 官方通常更快（实测 1.6MB/s vs 镜像 0.6~2MB/s），改为官方优先；
/// BMCLAPI 覆盖的 maven / Mojang 资源官方在国内常常几乎不通（forge maven 实测 1KB/s），
/// Modrinth CDN 也经常连不上，这些都必须镜像优先。
pub fn prefer_mirror_first(url: &str) -> bool {
    !(url.starts_with("https://edge.forgecdn.net/")
        || url.starts_with("https://mediafilez.forgecdn.net/")
        || url.starts_with("https://media.forgecdn.net/"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn rewrites_neoforged_maven_to_bmclapi() {
        let url = "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.259/neoforge-21.1.259-installer.jar";
        assert_eq!(
            super::rewrite(url),
            "https://bmclapi2.bangbang93.com/maven/net/neoforged/neoforge/21.1.259/neoforge-21.1.259-installer.jar"
        );
    }

    /// Forge 的 /releases/ 是仓库根路径的别名，镜像地址不能带这段
    #[test]
    fn rewrites_forge_releases_path_without_duplicating() {
        let url = "https://maven.minecraftforge.net/releases/net/minecraftforge/forge/1.20.1-47.4.20/forge-1.20.1-47.4.20-installer.jar";
        assert_eq!(
            super::rewrite(url),
            "https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/1.20.1-47.4.20/forge-1.20.1-47.4.20-installer.jar"
        );
    }

    /// Modrinth 接口也要能落到 MCIM 镜像
    #[test]
    fn rewrites_modrinth_api_to_mcim() {
        assert_eq!(
            super::rewrite("https://api.modrinth.com/v2/search?query=sodium"),
            "https://mod.mcimirror.top/modrinth/v2/search?query=sodium"
        );
    }
}
