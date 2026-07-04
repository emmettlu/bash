//! 轻量时间格式化工具, 基于 `nanotime`。

use nanotime::NanoTime;

const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(crate) fn format_time(
    time: &NanoTime,
    format: &crate::parser::prompt::PromptTimeFormat,
) -> String {
    match format {
        crate::parser::prompt::PromptTimeFormat::TwelveHourAM => {
            format!(
                "{:02}:{:02} {}",
                twelve_hour(time.hour()),
                time.minute(),
                am_pm(time.hour())
            )
        }
        crate::parser::prompt::PromptTimeFormat::TwelveHourHHMMSS => {
            format!(
                "{:02}:{:02}:{:02}",
                twelve_hour(time.hour()),
                time.minute(),
                time.second()
            )
        }
        crate::parser::prompt::PromptTimeFormat::TwentyFourHourHHMM => {
            format!("{:02}:{:02}", time.hour(), time.minute())
        }
        crate::parser::prompt::PromptTimeFormat::TwentyFourHourHHMMSS => {
            format!(
                "{:02}:{:02}:{:02}",
                time.hour(),
                time.minute(),
                time.second()
            )
        }
    }
}

pub(crate) fn format_date(
    time: &NanoTime,
    format: &crate::parser::prompt::PromptDateFormat,
) -> String {
    match format {
        crate::parser::prompt::PromptDateFormat::WeekdayMonthDate => {
            format!(
                "{} {} {:02}",
                weekday_short(time),
                month_short(time.month()),
                time.day()
            )
        }
        crate::parser::prompt::PromptDateFormat::Custom(format) => {
            format_strftime_subset(time, format)
        }
    }
}

pub(crate) fn format_strftime_subset(time: &NanoTime, format: &str) -> String {
    let mut result = String::new();
    let mut chars = format.chars();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            result.push(ch);
            continue;
        }

        let Some(spec) = chars.next() else {
            result.push('%');
            break;
        };

        match spec {
            '%' => result.push('%'),
            'Y' => result.push_str(&format!("{:04}", time.year())),
            'y' => result.push_str(&format!("{:02}", time.year() % 100)),
            'm' => result.push_str(&format!("{:02}", time.month())),
            'd' => result.push_str(&format!("{:02}", time.day())),
            'e' => result.push_str(&format!("{:2}", time.day())),
            'H' => result.push_str(&format!("{:02}", time.hour())),
            'I' => result.push_str(&format!("{:02}", twelve_hour(time.hour()))),
            'M' => result.push_str(&format!("{:02}", time.minute())),
            'S' => result.push_str(&format!("{:02}", time.second())),
            'p' => result.push_str(am_pm(time.hour())),
            'a' => result.push_str(weekday_short(time)),
            'b' | 'h' => result.push_str(month_short(time.month())),
            'F' => result.push_str(&format!(
                "{:04}-{:02}-{:02}",
                time.year(),
                time.month(),
                time.day()
            )),
            'R' => result.push_str(&format!("{:02}:{:02}", time.hour(), time.minute())),
            'T' => result.push_str(&format!(
                "{:02}:{:02}:{:02}",
                time.hour(),
                time.minute(),
                time.second()
            )),
            'f' => result.push_str(&format!("{:09}", time.nanosecond())),
            other => {
                result.push('%');
                result.push(other);
            }
        }
    }

    result
}

fn twelve_hour(hour: u8) -> u8 {
    match hour % 12 {
        0 => 12,
        hour => hour,
    }
}

fn am_pm(hour: u8) -> &'static str {
    if hour < 12 { "AM" } else { "PM" }
}

fn month_short(month: u8) -> &'static str {
    MONTHS_SHORT
        .get(usize::from(month.saturating_sub(1)))
        .copied()
        .unwrap_or("")
}

fn weekday_short(time: &NanoTime) -> &'static str {
    let days_since_epoch = time.to_epoch_secs() / 86_400;
    let weekday = (days_since_epoch + 4) % 7;
    WEEKDAYS_SHORT[usize::try_from(weekday).unwrap_or(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_time() -> NanoTime {
        NanoTime::new(2024, 12, 25, 13, 34, 56, 789_000_000).unwrap()
    }

    #[test]
    fn prompt_time_formats() {
        let time = test_time();
        assert_eq!(
            format_time(
                &time,
                &crate::parser::prompt::PromptTimeFormat::TwelveHourAM
            ),
            "01:34 PM"
        );
        assert_eq!(
            format_time(
                &time,
                &crate::parser::prompt::PromptTimeFormat::TwentyFourHourHHMMSS
            ),
            "13:34:56"
        );
        assert_eq!(
            format_time(
                &time,
                &crate::parser::prompt::PromptTimeFormat::TwelveHourHHMMSS
            ),
            "01:34:56"
        );
    }

    #[test]
    fn prompt_date_formats() {
        let time = test_time();
        assert_eq!(
            format_date(
                &time,
                &crate::parser::prompt::PromptDateFormat::WeekdayMonthDate
            ),
            "Wed Dec 25"
        );
        assert_eq!(
            format_date(
                &time,
                &crate::parser::prompt::PromptDateFormat::Custom(String::from("%Y-%m-%d"))
            ),
            "2024-12-25"
        );
        assert_eq!(
            format_date(
                &time,
                &crate::parser::prompt::PromptDateFormat::Custom(String::from(
                    "%Y-%m-%d %H:%M:%S.%f"
                ))
            ),
            "2024-12-25 13:34:56.789000000"
        );
    }
}
