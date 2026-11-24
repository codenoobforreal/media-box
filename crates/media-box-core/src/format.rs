//! 时长与 UTC 时间戳的格式化，不引入时间库。

use std::time::Duration;

/// 把时长格式化为 dd:hh:mm:ss，亚秒被丢弃。
pub fn format_duration_fixed(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}:{:02}", days, hours, minutes, seconds)
}

/// 把 Unix 时间戳格式化为 UTC 的 `yymmddHHMMSS`，避免为输出文件名引入时间库。
pub fn format_unix_timestamp_compact(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!(
        "{:02}{:02}{:02}{:02}{:02}{:02}",
        year % 100,
        month,
        day,
        hour,
        minute,
        second
    )
}

/// 由「1970-01-01 起的天数」还原公历日期（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_duration_as_dd_hh_mm_ss() {
        assert_eq!(format_duration_fixed(Duration::from_secs(0)), "00:00:00:00");
        // 1h 2m 5s
        assert_eq!(
            format_duration_fixed(Duration::from_secs(3_725)),
            "00:01:02:05"
        );
        // 1d 1h
        assert_eq!(
            format_duration_fixed(Duration::from_secs(90_000)),
            "01:01:00:00"
        );
        // 亚秒被丢弃
        assert_eq!(
            format_duration_fixed(Duration::from_millis(1_500)),
            "00:00:00:01"
        );
    }

    #[test]
    fn formats_unix_timestamp_across_calendar_boundaries() {
        // 纪元：1970-01-01 00:00:00 UTC
        assert_eq!(format_unix_timestamp_compact(0), "700101000000");

        // 闰年 2 月 29 日：2000-02-29 00:00:00 UTC
        assert_eq!(format_unix_timestamp_compact(951_782_400), "000229000000");

        // 年末最后一天：2024-12-31 23:59:59 UTC
        assert_eq!(format_unix_timestamp_compact(1_735_689_599), "241231235959");

        // 世纪年非闰年（2100 不能被 400 整除）：2100-03-01 00:00:00 UTC，两位年份为 00
        assert_eq!(format_unix_timestamp_compact(4_107_542_400), "000301000000");
    }
}
