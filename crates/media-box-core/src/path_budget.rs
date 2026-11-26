//! 文件名长度预算：按 UTF-16 码元截断，并在截断时附短哈希避免碰撞。
//!
//! Windows 与 macOS 都以 255 个 UTF-16 码元作为单组件上限；这里统一按该口径
//! 处理，避免依赖 `NAME_MAX` 的字节语义。整路径上限按平台区分。

use std::ffi::OsStr;
use std::path::Path;

/// 单组件上限（UTF-16 码元）：Windows NTFS/exFAT 与 macOS HFS+/APFS 一致。
pub const MAX_COMPONENT_UTF16: usize = 255;

/// 截断后追加的哈希后缀长度：`-` + 16 位十六进制。
const HASH_SUFFIX_UTF16: usize = 17;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 文件名预算错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathBudgetError {
    /// 预留后缀后已无空间容纳截断哈希。
    #[error("文件名预算不足：后缀需占 {reserved} 个码元，无法在 {max} 个码元内保留哈希")]
    BudgetTooSmall {
        /// 调用方预留的码元数。
        reserved: usize,
        /// 单组件上限。
        max: usize,
    },
    /// 单个路径组件超过上限。
    #[error("文件名组件超过 {max} 个 UTF-16 码元：{component}（{actual}）")]
    ComponentTooLong {
        /// 超长的组件文本。
        component: String,
        /// 实际码元数。
        actual: usize,
        /// 单组件上限。
        max: usize,
    },
    /// 整条路径超过平台预算。
    #[error("路径长度超过 {limit} 个 UTF-16 码元：{path}（{actual}）")]
    PathTooLong {
        /// 超长的路径文本。
        path: String,
        /// 实际码元数。
        actual: usize,
        /// 平台预算。
        limit: usize,
    },
}

/// 为后续后缀预留 `reserved` 个码元，必要时截断 `stem` 并附短哈希。
///
/// 未超预算时原样返回；超预算时保留前 `255 - reserved - 17` 个码元，
/// 再追加 FNV-1a 哈希，保证不同长名不会因截断而互相覆盖。
#[must_use]
pub fn safe_output_stem(stem: &str, reserved: usize) -> Result<String, PathBudgetError> {
    let budget =
        MAX_COMPONENT_UTF16
            .checked_sub(reserved)
            .ok_or(PathBudgetError::BudgetTooSmall {
                reserved,
                max: MAX_COMPONENT_UTF16,
            })?;
    if budget < HASH_SUFFIX_UTF16 {
        return Err(PathBudgetError::BudgetTooSmall {
            reserved,
            max: MAX_COMPONENT_UTF16,
        });
    }

    if utf16_units(stem) <= budget {
        return Ok(stem.to_owned());
    }

    let prefix = truncate_utf16_prefix(stem, budget - HASH_SUFFIX_UTF16);
    Ok(format!("{prefix}-{:016x}", fnv1a(stem.as_bytes())))
}

/// 校验路径的每个组件与整路径长度是否落在平台预算内。
pub fn validate_output_path(path: &Path) -> Result<(), PathBudgetError> {
    for component in path.components() {
        let text = component.as_os_str();
        let units = os_str_utf16_units(text);
        if units > MAX_COMPONENT_UTF16 {
            return Err(PathBudgetError::ComponentTooLong {
                component: text.to_string_lossy().into_owned(),
                actual: units,
                max: MAX_COMPONENT_UTF16,
            });
        }
    }

    let actual = os_str_utf16_units(path.as_os_str());
    let limit = max_path_units(path);
    if actual > limit {
        return Err(PathBudgetError::PathTooLong {
            path: path.to_string_lossy().into_owned(),
            actual,
            limit,
        });
    }
    Ok(())
}

fn utf16_units(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(windows)]
fn os_str_utf16_units(text: &OsStr) -> usize {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().count()
}

#[cfg(not(windows))]
fn os_str_utf16_units(text: &OsStr) -> usize {
    text.to_string_lossy().encode_utf16().count()
}

#[cfg(windows)]
fn max_path_units(path: &Path) -> usize {
    use std::path::{Component, Prefix};
    let verbatim = matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(
            prefix.kind(),
            Prefix::Verbatim(_) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..)
        )
    );
    // 260 含终止 null；扩展长度路径 32767 也含终止 null，各留一位。
    if verbatim { 32_766 } else { 259 }
}

#[cfg(target_os = "macos")]
fn max_path_units(path: &Path) -> usize {
    let _ = path;
    1_023
}

#[cfg(not(any(windows, target_os = "macos")))]
fn max_path_units(path: &Path) -> usize {
    let _ = path;
    4_095
}

fn truncate_utf16_prefix(text: &str, max_units: usize) -> &str {
    let mut used = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let width = character.len_utf16();
        if used + width > max_units {
            break;
        }
        used += width;
        end = index + character.len_utf8();
    }
    &text[..end]
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_stem_is_unchanged() {
        assert_eq!(
            safe_output_stem("photo", 20).unwrap(),
            "photo",
            "未超预算不应改动文件名"
        );
    }

    #[test]
    fn ascii_stem_is_truncated_with_hash_within_budget() {
        let long = "a".repeat(300);
        let result = safe_output_stem(&long, 28).unwrap();
        assert_eq!(utf16_units(&result), 255 - 28, "应精确用满预算");
        assert!(result.ends_with(&format!("-{:016x}", fnv1a(long.as_bytes()))));
    }

    #[test]
    fn cjk_and_emoji_stems_never_split_a_surrogate_pair() {
        for long in ["汉".repeat(200), "🚧".repeat(120)] {
            let result = safe_output_stem(&long, 28).unwrap();
            assert!(utf16_units(&result) <= MAX_COMPONENT_UTF16 - 28);
            assert!(result.is_char_boundary(result.len()), "不能切断码点");
            assert!(!result.contains('\u{FFFD}'), "不能产生替换字符");
        }
    }

    #[test]
    fn different_long_stems_get_different_hashes() {
        let left = safe_output_stem(&"a".repeat(300), 28).unwrap();
        let right = safe_output_stem(&"b".repeat(300), 28).unwrap();
        assert_ne!(left, right, "截断后的名字不能互相覆盖");
    }

    #[test]
    fn rejects_component_over_255_utf16_units() {
        let name = format!("{}.txt", "a".repeat(256));
        let path = Path::new(&name);
        assert!(matches!(
            validate_output_path(path),
            Err(PathBudgetError::ComponentTooLong { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_relative_path_over_max_path() {
        let name = format!("{}/movie.txt", "d".repeat(252));
        let path = Path::new(&name);
        assert!(matches!(
            validate_output_path(path),
            Err(PathBudgetError::PathTooLong { .. })
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_path_over_1023_units() {
        let name = format!("{}/movie.txt", "d".repeat(1000));
        let path = Path::new(&name);
        assert!(matches!(
            validate_output_path(path),
            Err(PathBudgetError::PathTooLong { .. })
        ));
    }
}
