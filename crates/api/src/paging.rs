//! Reading a list a page at a time.

/// The most rows one page holds, whatever is asked.
const MOST: usize = 200;

/// How many rows a page holds: what was asked, or `default` when nothing was.
pub(crate) fn page_limit(asked: i32, default: usize) -> usize {
    match asked {
        n if n <= 0 => default,
        n => (n as usize).min(MOST),
    }
}

/// The page out of `rows`, read one past `limit`, and whether older ones
/// remain.
pub(crate) fn page<T>(mut rows: Vec<T>, limit: usize) -> (Vec<T>, bool) {
    let more = rows.len() > limit;
    rows.truncate(limit);
    (rows, more)
}
