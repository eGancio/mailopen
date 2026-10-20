use chrono::{Datelike, Local, TimeZone};

const MONTHS: [&str; 12] = [
    "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
];
const DAYS: [&str; 7] = ["lun", "mar", "mer", "gio", "ven", "sab", "dom"];

fn month(m: u32) -> &'static str {
    MONTHS
        .get(m.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("")
}

/// "10:56" today, "3 ott" this year, "03/10/25" otherwise. Empty when unknown.
pub fn short_date(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if t.year() == now.year() {
        format!("{} {}", t.day(), month(t.month()))
    } else {
        t.format("%d/%m/%y").to_string()
    }
}

/// "lun 20 ott 2026, 10:56".
pub fn long_date(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let dow = DAYS
        .get(t.weekday().num_days_from_monday() as usize)
        .copied()
        .unwrap_or("");
    format!(
        "{dow} {} {} {}, {}",
        t.day(),
        month(t.month()),
        t.year(),
        t.format("%H:%M")
    )
}

/// "1,2 MB", "340 KB", "900 B".
pub fn size(bytes: u32) -> String {
    let b = f64::from(bytes);
    if b >= 1_048_576.0 {
        format!("{:.1} MB", b / 1_048_576.0).replace('.', ",")
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Prefix every line with "> " for quoting in replies.
pub fn quote(text: &str) -> String {
    text.lines()
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_quotes() {
        assert_eq!(size(900), "900 B");
        assert_eq!(size(340 * 1024), "340 KB");
        assert_eq!(size(1_300_000), "1,2 MB");
        assert_eq!(quote("a\nb"), "> a\n> b");
        assert_eq!(short_date(0), "");
        assert!(!short_date(1_700_000_000).is_empty());
    }
}
