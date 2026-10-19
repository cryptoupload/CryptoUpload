// SPDX-License-Identifier: GPL-3.0-or-later

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

fn to_unix(year: i64, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> Option<i64> {
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || min > 59
        || sec > 60
    {
        return None;
    }
    Some(
        days_from_civil(year, month, day) * 86_400
            + i64::from(hour) * 3600
            + i64::from(min) * 60
            + i64::from(sec.min(59)),
    )
}

fn num<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[must_use]
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-');
    let year: i64 = num(d.next()?)?;
    let month: u32 = num(d.next()?)?;
    let day: u32 = num(d.next()?)?;

    let (time, offset_secs) = if let Some(t) = rest.strip_suffix(['Z', 'z']) {
        (t, 0)
    } else {
        let idx = rest.rfind(['+', '-'])?;
        let (t, off) = rest.split_at(idx);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..].split_once(':')?;
        let oh: i64 = num(oh)?;
        let om: i64 = num(om)?;
        (t, sign * (oh * 3600 + om * 60))
    };
    let time = time.split_once('.').map_or(time, |(whole, _frac)| whole);
    let mut t = time.splitn(3, ':');
    let hour: u32 = num(t.next()?)?;
    let min: u32 = num(t.next()?)?;
    let sec: u32 = num(t.next()?)?;
    Some(to_unix(year, month, day, hour, min, sec)? - offset_secs)
}

#[must_use]
pub fn parse_http_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let (_weekday, rest) = s.split_once(", ")?;
    let mut parts = rest.split(' ');
    let day: u32 = num(parts.next()?)?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = num(parts.next()?)?;
    let mut t = parts.next()?.splitn(3, ':');
    let hour: u32 = num(t.next()?)?;
    let min: u32 = num(t.next()?)?;
    let sec: u32 = num(t.next()?)?;
    if parts.next()? != "GMT" || parts.next().is_some() {
        return None;
    }
    to_unix(year, month, day, hour, min, sec)
}
