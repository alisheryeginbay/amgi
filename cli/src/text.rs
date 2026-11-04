//! Times, widths and small drawings for terminal output.

use std::time::{SystemTime, UNIX_EPOCH};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn local(t: i64) -> libc::tm {
    // SAFETY: `localtime_r` fills the zeroed `tm` it's given and keeps no pointer to it.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&(t as libc::time_t), &mut tm);
        tm
    }
}

/// Seconds east of UTC in the local time zone at `t`.
pub fn utc_offset(t: i64) -> i32 {
    local(t).tm_gmtoff as i32
}

/// `2026-10-20 14:02`, local time.
pub fn date_time(t: i64) -> String {
    let tm = local(t);
    format!("{} {:02}:{:02}", date(t), tm.tm_hour, tm.tm_min)
}

/// `2026-10-20`, local time.
pub fn date(t: i64) -> String {
    let tm = local(t);
    format!("{}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/// `Mon`, local time.
pub fn weekday(t: i64) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][local(t).tm_wday.rem_euclid(7) as usize]
}

/// The day of the week at `t`, local time, Monday being 0.
pub fn weekday_index(t: i64) -> usize {
    (local(t).tm_wday + 6).rem_euclid(7) as usize
}

/// `Jan`, local time.
pub fn month(t: i64) -> &'static str {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    MONTHS[local(t).tm_mon.rem_euclid(12) as usize]
}

/// Time spent studying, rounded to what matters at its size: `45 s`, `23 min`,
/// `1 h 5 min`. The app writes it the same way.
pub fn study_time(secs: u32) -> String {
    if secs < 60 {
        return format!("{secs} s");
    }
    let minutes = (secs + 30) / 60;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    match (minutes / 60, minutes % 60) {
        (hours, 0) => format!("{hours} h"),
        (hours, rest) => format!("{hours} h {rest} min"),
    }
}

/// Seconds per review: a decimal under ten, `8.4 s`, where whole seconds are too coarse.
pub fn review_time(secs: f64) -> String {
    if secs < 9.95 { format!("{secs:.1} s") } else { study_time(secs.round() as u32) }
}

/// Columns of the terminal on stdout, or `None` when piped.
pub fn terminal_width() -> Option<usize> {
    // SAFETY: TIOCGWINSZ only writes the `winsize` it's given.
    unsafe {
        if libc::isatty(libc::STDOUT_FILENO) == 0 {
            return None;
        }
        let mut size: libc::winsize = std::mem::zeroed();
        (libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0)
            .then_some(size.ws_col as usize)
    }
}

/// A card side without the markers the core puts around a gap inside code.
pub fn plain(s: &str) -> String {
    s.replace([amgi_core::parser::CODE_GAP_OPEN, amgi_core::parser::CODE_GAP_CLOSE], "")
}

/// Markdown on one line: runs of whitespace, newlines included, become one space.
pub fn one_line(s: &str) -> String {
    plain(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cut to `width` columns, ending in `…` if anything was cut.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Padded with spaces to `width` columns, counting wide characters such as 한 as two.
pub fn pad(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(s.width())))
}

pub fn width(s: &str) -> usize {
    s.width()
}

/// One block per value, scaled to the largest: `▁▃█`.
pub fn sparkline(values: &[u32]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    values
        .iter()
        .map(|&v| if v == 0 { ' ' } else { BLOCKS[((v * 8 - 1) / max).min(7) as usize] })
        .collect()
}

/// How dark a day is in the calendar, 0 for none and 1–4 by its quartile among the
/// days that have any, so one long day doesn't wash out the rest.
pub fn levels(values: &[u32]) -> Vec<usize> {
    let mut active: Vec<u32> = values.iter().copied().filter(|&v| v > 0).collect();
    active.sort_unstable();
    let bounds: Vec<u32> = if active.is_empty() {
        vec![]
    } else {
        [0.25, 0.5, 0.75].iter().map(|q| active[((active.len() - 1) as f64 * q) as usize]).collect()
    };
    values.iter().map(|&v| if v == 0 { 0 } else { 1 + bounds.iter().filter(|&&b| v > b).count() }).collect()
}

/// A bar `width` columns long when `value` is `max`.
pub fn bar(value: u32, max: u32, width: usize) -> String {
    let len = (value as usize * width).div_ceil(max.max(1) as usize);
    "█".repeat(len)
}

/// `1 card`, `3 cards`.
pub fn count(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `840 bytes`, `12 KB`, `3.4 MB`, in decimal units as Finder shows them.
pub fn size(bytes: u64) -> String {
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    if value < 1000.0 {
        return count(bytes as u32, "byte", "bytes");
    }
    for unit in units {
        value /= 1000.0;
        if value < 999.95 || unit == "TB" {
            return if value < 10.0 { format!("{value:.1} {unit}") } else { format!("{value:.0} {unit}") };
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn study_and_review_times() {
        assert_eq!(study_time(45), "45 s");
        assert_eq!(study_time(23 * 60 + 10), "23 min");
        assert_eq!(study_time(3600), "1 h");
        assert_eq!(study_time(3900), "1 h 5 min");
        assert_eq!(review_time(8.44), "8.4 s");
        assert_eq!(review_time(12.4), "12 s");
    }

    #[test]
    fn levels_by_quartile() {
        assert_eq!(levels(&[0, 1, 2, 3, 4, 100]), [0, 1, 1, 2, 3, 4]);
        assert_eq!(levels(&[0, 0]), [0, 0]);
    }
}
