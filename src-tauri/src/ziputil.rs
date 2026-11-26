//! 容错的 zip 打开逻辑。
//!
//! 少数重打包工具会在 0x7075「Unicode 文件名」扩展字段里留下过期的 CRC32。
//! Windows 资源管理器、7-Zip、.NET 都会忽略这个字段，但 `zip` crate 把它
//! 当成致命错误，于是整个压缩包被判为损坏（表现为整合包完全无法识别）。
//!
//! 这里在直接打开失败时，把这类字段的 ID 改成未使用值（长度不变，不动任何
//! 数据偏移），修出一份临时副本再交给 zip crate。只有 CRC 不匹配的字段会被
//! 改写，正常条目不受影响。

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

/// Info-ZIP Unicode Path Extra Field
const UNICODE_PATH_EXTRA: u16 = 0x7075;
/// 不会被识别的扩展字段 ID，用来让 zip crate 直接跳过
const UNUSED_EXTRA: u16 = 0xFFFF;
const LOCAL_HEADER_SIZE: u64 = 30;
const CENTRAL_HEADER_SIZE: usize = 46;
const EOCD_SIZE: usize = 22;

/// 打开压缩包；遇到 CRC 过期的 Unicode 名字扩展字段时自动修复后重试。
pub fn open(path: &Path) -> Result<ZipArchive<File>> {
    let file = File::open(path).with_context(|| format!("无法打开压缩包：{}", path.display()))?;
    match ZipArchive::new(file) {
        Ok(archive) => Ok(archive),
        Err(error) => {
            if let Some(repaired) = repair(path) {
                if let Ok(file) = File::open(&repaired) {
                    if let Ok(archive) = ZipArchive::new(file) {
                        return Ok(archive);
                    }
                }
            }
            Err(anyhow::Error::new(error).context("压缩包损坏或格式不支持"))
        }
    }
}

/// 生成修复副本；同一文件重复打开时复用上一次的结果。
fn repair(path: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    meta.len().hash(&mut hasher);
    if let Ok(modified) = meta.modified() {
        if let Ok(stamp) = modified.duration_since(std::time::UNIX_EPOCH) {
            stamp.as_secs().hash(&mut hasher);
        }
    }
    let dir = crate::config::data_dir().join("tmp");
    std::fs::create_dir_all(&dir).ok()?;
    let dest = dir.join(format!("zipfix-{:016x}.zip", hasher.finish()));
    if dest.is_file() {
        return Some(dest);
    }
    let part = dir.join(format!("zipfix-{:016x}.part", hasher.finish()));
    std::fs::copy(path, &part).ok()?;
    match patch(&part) {
        Ok(true) => {
            // 副本写完再改名，避免并发打开时读到半成品
            if std::fs::rename(&part, &dest).is_ok() {
                Some(dest)
            } else {
                None
            }
        }
        _ => {
            let _ = std::fs::remove_file(&part);
            None
        }
    }
}

/// 就地修补中央目录与本地头里的过期扩展字段；返回是否改过。
fn patch(path: &Path) -> Result<bool> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    if len < EOCD_SIZE as u64 {
        return Ok(false);
    }
    // 尾部找 EOCD：最多需要看 22 字节 + 65535 字节注释
    let tail_len = len.min(66_000);
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    let Some(eocd) = rfind(&tail, b"PK\x05\x06") else {
        return Ok(false);
    };
    if eocd + EOCD_SIZE > tail.len() {
        return Ok(false);
    }
    let cd_size = u32::from_le_bytes(tail[eocd + 12..eocd + 16].try_into()?) as usize;
    let cd_offset = u32::from_le_bytes(tail[eocd + 16..eocd + 20].try_into()?) as u64;
    // ZIP64 或异常偏移不处理，交给上层报原错误
    if cd_size == 0 || cd_offset == u32::MAX as u64 {
        return Ok(false);
    }
    if cd_offset.saturating_add(cd_size as u64) > len {
        return Ok(false);
    }

    let mut central = vec![0u8; cd_size];
    file.seek(SeekFrom::Start(cd_offset))?;
    file.read_exact(&mut central)?;

    let mut changed_cd = false;
    let mut local_headers: Vec<u64> = Vec::new();
    let mut pos = 0usize;
    while pos + CENTRAL_HEADER_SIZE <= central.len() {
        if &central[pos..pos + 4] != b"PK\x01\x02" {
            return Ok(false);
        }
        let name_len = u16::from_le_bytes([central[pos + 28], central[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([central[pos + 30], central[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([central[pos + 32], central[pos + 33]]) as usize;
        let local_offset = u32::from_le_bytes(central[pos + 42..pos + 46].try_into()?) as u64;
        let name_start = pos + CENTRAL_HEADER_SIZE;
        let extra_start = name_start + name_len;
        let next = extra_start + extra_len + comment_len;
        if next > central.len() {
            return Ok(false);
        }
        if extra_len > 0 {
            let name = central[name_start..name_start + name_len].to_vec();
            if patch_extra(&mut central[extra_start..extra_start + extra_len], &name) {
                changed_cd = true;
                local_headers.push(local_offset);
            }
        }
        pos = next;
    }
    if changed_cd {
        file.seek(SeekFrom::Start(cd_offset))?;
        file.write_all(&central)?;
    }
    // 本地头里也常带同样的字段，抽取条目时会被解析，必须一起修
    for offset in local_headers {
        if patch_local(&mut file, offset).is_err() {
            return Ok(false);
        }
    }
    Ok(changed_cd)
}

/// 把 CRC 与真实文件名不符的 Unicode 名字字段改成未使用 ID。
fn patch_extra(extra: &mut [u8], name: &[u8]) -> bool {
    let actual = crc32fast::hash(name);
    let mut pos = 0usize;
    let mut changed = false;
    while pos + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[pos], extra[pos + 1]]);
        let size = u16::from_le_bytes([extra[pos + 2], extra[pos + 3]]) as usize;
        if pos + 4 + size > extra.len() {
            break;
        }
        // 结构：版本(1) + CRC32(4) + 名字字节
        if id == UNICODE_PATH_EXTRA && size >= 5 {
            let stored = u32::from_le_bytes(extra[pos + 5..pos + 9].try_into().unwrap_or([0; 4]));
            if stored != actual {
                extra[pos..pos + 2].copy_from_slice(&UNUSED_EXTRA.to_le_bytes());
                changed = true;
            }
        }
        pos += 4 + size;
    }
    changed
}

fn patch_local(file: &mut File, offset: u64) -> Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; LOCAL_HEADER_SIZE as usize];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"PK\x03\x04" {
        return Ok(());
    }
    let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
    let extra_len = u16::from_le_bytes([header[28], header[29]]) as usize;
    let mut name = vec![0u8; name_len];
    file.read_exact(&mut name)?;
    if extra_len == 0 {
        return Ok(());
    }
    let mut extra = vec![0u8; extra_len];
    file.read_exact(&mut extra)?;
    if patch_extra(&mut extra, &name) {
        file.seek(SeekFrom::Start(
            offset + LOCAL_HEADER_SIZE + name_len as u64,
        ))?;
        file.write_all(&extra)?;
    }
    Ok(())
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::open;
    use std::io::{Read, Write};
    use std::path::PathBuf;

    fn temp_path(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("docl-zipfix-{label}-{stamp}.zip"))
    }

    /// 手工拼一个带过期 0x7075 字段的 zip：zip crate 会拒绝，容错打开要能读出来
    #[test]
    fn repairs_stale_unicode_path_extra() {
        let name = "配置说明.txt".as_bytes();
        let data = b"hello";
        let crc = crc32fast::hash(data);
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x7075u16.to_le_bytes());
        extra.extend_from_slice(&((name.len() + 5) as u16).to_le_bytes());
        extra.push(1); // 版本
        extra.extend_from_slice(&0u32.to_le_bytes()); // 故意写错的 CRC
        extra.extend_from_slice(name);

        let mut out = Vec::new();
        let local_offset = 0u32;
        // 本地头
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x800u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&extra);
        out.extend_from_slice(data);

        let cd_offset = out.len() as u32;
        // 中央目录
        out.extend_from_slice(b"PK\x01\x02");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x800u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&local_offset.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&extra);
        let cd_size = (out.len() as u32) - cd_offset;

        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());

        let path = temp_path("stale");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&out)
            .unwrap();

        // 原始 zip crate 打不开
        let file = std::fs::File::open(&path).unwrap();
        assert!(zip::ZipArchive::new(file).is_err());

        let mut archive = open(&path).expect("容错打开应成功");
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name(), "配置说明.txt");
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello");
        drop(entry);
        drop(archive);
        let _ = std::fs::remove_file(&path);
    }

    /// 正常 zip 不应触发修复流程
    #[test]
    fn opens_valid_archive_directly() {
        let path = temp_path("valid");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("a.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"ok").unwrap();
        writer.finish().unwrap();

        let archive = open(&path).unwrap();
        assert_eq!(archive.len(), 1);
        drop(archive);
        let _ = std::fs::remove_file(&path);
    }
}
