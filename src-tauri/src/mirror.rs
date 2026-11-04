/// BMCLAPI 镜像地址重写。
/// 文档：https://bmclapidoc.bangbang93.com/
const MIRROR: &str = "https://bmclapi2.bangbang93.com";

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
}

pub fn official(url: &str) -> String {
    if let Some(rest) = url.strip_prefix(MIRROR) {
        // 还原为官方地址；多数资源可直接用官方主机
        if rest.starts_with("/assets/") {
            return format!(
                "https://resources.download.minecraft.net/{}",
                rest.trim_start_matches("/assets/")
            );
        }
        if rest.starts_with("/maven/") {
            return format!(
                "https://libraries.minecraft.net/{}",
                rest.trim_start_matches("/maven/")
            );
        }
        return format!("https://launchermeta.mojang.com{rest}");
    }
    url.to_string()
}
