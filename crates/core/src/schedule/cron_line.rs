//! The lines a routine's time is written in.

use std::str::FromStr;

/// Read a five-field cron expression. The `cron` crate wants a seconds field
/// first, so one is added.
pub fn parse_cron(expr: &str) -> anyhow::Result<cron::Schedule> {
    let fields = expr.split_whitespace().count();
    anyhow::ensure!(
        fields == 5,
        "a schedule needs five fields (minute hour day month weekday): {expr}"
    );
    let mut parts: Vec<String> = expr.split_whitespace().map(str::to_owned).collect();
    parts[4] = weekdays(&parts[4]);
    cron::Schedule::from_str(&format!("0 {}", parts.join(" ")))
        .map_err(|e| anyhow::anyhow!("invalid schedule {expr:?}: {e}"))
}

/// The weekday field with its numbers as names: the `cron` crate counts from
/// Sunday as 1, where a crontab has Sunday as 0 or 7.
fn weekdays(field: &str) -> String {
    const NAMES: [&str; 8] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let name = |n: &str| -> String {
        n.parse::<usize>()
            .ok()
            .and_then(|n| NAMES.get(n))
            .map_or_else(|| n.to_owned(), |n| (*n).to_owned())
    };
    field
        .split(',')
        .flat_map(|part| {
            let (days, step) = match part.split_once('/') {
                Some((days, step)) => (days, format!("/{step}")),
                None => (part, String::new()),
            };
            match days.split_once('-') {
                // A range up to Sunday wraps round the week's start.
                Some((from, "7")) if from != "0" => {
                    vec![format!("{}-Sat{step}", name(from)), "Sun".to_owned()]
                }
                Some((from, to)) => vec![format!("{}-{}{step}", name(from), name(to))],
                None => vec![format!("{}{step}", name(days))],
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The cron line for a time picked in words: `repeat` is daily, weekdays,
/// weekly or monthly, `time` is HH:MM, `weekday` 0 (Sunday) to 6, `day` 1 to
/// 28. Without a repeat, `cron` is taken as written.
pub fn cron_from(
    repeat: &str,
    time: &str,
    weekday: i32,
    day: i32,
    cron: &str,
) -> anyhow::Result<String> {
    if repeat.is_empty() || repeat == "cron" {
        anyhow::ensure!(!cron.trim().is_empty(), "write the time as cron");
        parse_cron(cron.trim())?;
        return Ok(cron.trim().to_owned());
    }
    let (h, m) = time
        .split_once(':')
        .and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)))
        .filter(|(h, m)| *h < 24 && *m < 60)
        .ok_or_else(|| anyhow::anyhow!("pick a time as HH:MM"))?;
    Ok(match repeat {
        "daily" => format!("{m} {h} * * *"),
        "weekdays" => format!("{m} {h} * * 1-5"),
        "weekly" => format!(
            "{m} {h} * * {}",
            if (0..7).contains(&weekday) {
                weekday
            } else {
                1
            }
        ),
        "monthly" => format!(
            "{m} {h} {} * *",
            if (1..=28).contains(&day) { day } else { 1 }
        ),
        other => anyhow::bail!("unknown repeat {other}"),
    })
}

/// A cron line in words, for the ones [`cron_from`] makes; others as they are.
pub fn describe_cron(cron: &str) -> String {
    const DAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];
    let f: Vec<&str> = cron.split_whitespace().collect();
    let [m, h, dom, mon, dow] = f[..] else {
        return cron.to_owned();
    };
    let (Ok(m), Ok(h)) = (m.parse::<u32>(), h.parse::<u32>()) else {
        return cron.to_owned();
    };
    let at = format!("{h}:{m:02}");
    match (dom, mon, dow) {
        ("*", "*", "*") => format!("毎日 {at}"),
        ("*", "*", "1-5") => format!("平日 {at}"),
        ("*", "*", d) if d.parse::<usize>().is_ok_and(|d| d < 7) => {
            format!("毎週{}曜 {at}", DAYS[d.parse::<usize>().unwrap_or(0)])
        }
        (d, "*", "*") if d.parse::<u32>().is_ok() => format!("毎月 {d} 日 {at}"),
        _ => cron.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use chrono_tz::Asia::Tokyo as TOKYO;

    use super::*;

    #[test]
    fn a_time_picked_in_words_becomes_cron_and_reads_back() {
        assert_eq!(
            cron_from("weekdays", "08:30", 0, 0, "").unwrap(),
            "30 8 * * 1-5"
        );
        assert_eq!(
            cron_from("weekly", "21:00", 0, 0, "").unwrap(),
            "0 21 * * 0"
        );
        assert_eq!(
            cron_from("monthly", "09:05", 0, 25, "").unwrap(),
            "5 9 25 * *"
        );
        assert_eq!(
            cron_from("", "", 0, 0, "*/5 * * * *").unwrap(),
            "*/5 * * * *"
        );
        assert!(cron_from("daily", "", 0, 0, "").is_err());
        assert_eq!(describe_cron("30 8 * * 1-5"), "平日 8:30");
        assert_eq!(describe_cron("0 21 * * 0"), "毎週日曜 21:00");
        assert_eq!(describe_cron("5 9 25 * *"), "毎月 25 日 9:05");
        assert_eq!(describe_cron("*/5 * * * *"), "*/5 * * * *");
    }

    #[test]
    fn a_schedule_needs_five_fields() {
        assert!(parse_cron("0 9 * * *").is_ok());
        assert!(parse_cron("0 0 9 * * *").is_err());
        assert!(parse_cron("every morning").is_err());
    }

    #[test]
    fn weekdays_are_counted_from_sunday_as_zero_or_seven() {
        let days = |cron: &str| -> Vec<String> {
            let from = TOKYO.with_ymd_and_hms(2026, 11, 22, 0, 0, 0).unwrap();
            parse_cron(cron)
                .unwrap()
                .after(&from)
                .take_while(|t| t.timestamp() < from.timestamp() + 7 * 86400)
                .map(|t| t.format("%a").to_string())
                .collect()
        };
        assert_eq!(days("0 9 * * 1-5"), ["Mon", "Tue", "Wed", "Thu", "Fri"]);
        assert_eq!(days("0 9 * * 0"), ["Sun"]);
        assert_eq!(days("0 9 * * 7"), ["Sun"]);
        assert_eq!(days("0 9 * * 6-7,1"), ["Sun", "Mon", "Sat"]);
        assert_eq!(days("0 9 * * Mon-Wed"), ["Mon", "Tue", "Wed"]);
    }
}
