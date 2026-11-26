//! 终端显示宽度估算与截断：CJK 与 emoji 按两列计。
//!
//! 不引入 `unicode-width` 依赖：这里只处理项目实际会显示的中文路径与常见 emoji，
//! 覆盖东亚宽字符、全角字符与 emoji 的主要区间；其余字符按一列计。

/// 估算字符占用的终端列数。
fn char_width(character: char) -> usize {
    let code = u32::from(character);
    if is_wide(code) { 2 } else { 1 }
}

/// 东亚宽字符与 emoji 的常见区间。
fn is_wide(code: u32) -> bool {
    matches!(code,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF
        | 0x20000..=0x3FFFD
    )
}

/// 字符串的终端显示宽度（列数）。
#[must_use]
pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 截断到最多 `max_width` 列，超限时用 `…` 结尾，不切断字符。
///
/// 返回值的显示宽度不超过 `max_width`；`max_width` 为 0 时返回空串。
#[must_use]
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }

    let limit = max_width - 1;
    let mut used = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let width = char_width(character);
        if used + width > limit {
            break;
        }
        used += width;
        end = index + character.len_utf8();
    }
    let mut result = String::with_capacity(end + 3);
    result.push_str(&text[..end]);
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_width_is_char_count() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(truncate_to_width("abcdef", 4), "abc…");
    }

    #[test]
    fn cjk_and_emoji_count_as_two_columns() {
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("🚧"), 2);
        assert_eq!(truncate_to_width("中文abc", 5), "中文…");
    }

    #[test]
    fn never_splits_a_character() {
        let result = truncate_to_width("🚧🚧🚧", 3);
        assert_eq!(result, "🚧…");
        assert_eq!(display_width(&result), 3);
    }

    #[test]
    fn short_text_is_unchanged() {
        assert_eq!(truncate_to_width("短", 10), "短");
        assert_eq!(truncate_to_width("", 0), "");
    }
}
