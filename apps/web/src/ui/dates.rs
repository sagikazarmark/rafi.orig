//! Dates in the browser: today as the organizer sees it, and the end of a
//! day where they are. Dates to show come from the server, in words.

/// Today, `YYYY-MM-DD`, in the browser's timezone.
pub fn today_iso() -> String {
    let d = js_sys::Date::new_0();
    format!(
        "{:04}-{:02}-{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date()
    )
}

/// When the day `iso` (`YYYY-MM-DD`) ends in the browser's timezone, in unix
/// seconds: the start of the next local day. `None` if `iso` isn't a date.
pub fn end_of_day_local(iso: &str) -> Option<i64> {
    let mut parts = iso.trim().split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    // `Date.new_with_year_month_day` normalizes day + 1 past a month's end.
    let next = js_sys::Date::new_with_year_month_day(y as u32, m - 1, d + 1);
    let ms = next.get_time();
    ms.is_finite().then(|| (ms / 1000.0) as i64)
}
