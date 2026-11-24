//! 字节数的人类可读格式化。

use std::fmt;

/// 字节数，显示时按 1024 进制自动换算为 KiB / MiB 等。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteSize(u64);

impl ByteSize {
    /// 由原始字节数构造。
    pub fn b(bytes: u64) -> Self {
        Self(bytes)
    }

    /// 返回原始字节数。
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

        let mut value = self.0 as f64;
        let mut unit = 0;

        while value >= 1024.0 && unit + 1 < UNITS.len() {
            value /= 1024.0;
            unit += 1;
        }

        if unit == 0 {
            write!(f, "{} {}", self.0, UNITS[unit])
        } else {
            write!(f, "{value:.2} {}", UNITS[unit])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_human_readable_units() {
        assert_eq!(ByteSize::b(0).to_string(), "0 B");
        assert_eq!(ByteSize::b(1023).to_string(), "1023 B");
        // 单位切换边界
        assert_eq!(ByteSize::b(1024).to_string(), "1.00 KiB");
        assert_eq!(ByteSize::b(1_048_575).to_string(), "1024.00 KiB");
        assert_eq!(ByteSize::b(1_048_576).to_string(), "1.00 MiB");
        assert_eq!(ByteSize::b(1_073_741_824).to_string(), "1.00 GiB");
        // 超出 TiB 不再升单位
        assert_eq!(
            ByteSize::b(1_125_899_906_842_624).to_string(),
            "1024.00 TiB"
        );
    }
}
