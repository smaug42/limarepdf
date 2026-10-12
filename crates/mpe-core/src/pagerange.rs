use crate::Result;
use anyhow::bail;

/// Parse a 1-based page range such as `1-3,5,8-` or `all` into sorted,
/// de-duplicated 0-based indices. `total` is the page count.
pub fn parse_page_range(spec: &str, total: usize) -> Result<Vec<usize>> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return Ok((0..total).collect());
    }
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => {
                let a = if a.trim().is_empty() { 1 } else { a.trim().parse::<usize>()? };
                let b = if b.trim().is_empty() { total } else { b.trim().parse::<usize>()? };
                (a, b)
            }
            None => {
                let n = part.parse::<usize>()?;
                (n, n)
            }
        };
        if a == 0 || b == 0 || a > b || b > total {
            bail!("page range '{part}' is outside 1..={total}");
        }
        out.extend((a - 1)..b);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_page_range("1-3,5", 10).unwrap(), vec![0, 1, 2, 4]);
        assert_eq!(parse_page_range("8-", 10).unwrap(), vec![7, 8, 9]);
        assert_eq!(parse_page_range("-2", 10).unwrap(), vec![0, 1]);
        assert_eq!(parse_page_range("all", 3).unwrap(), vec![0, 1, 2]);
        assert!(parse_page_range("11", 10).is_err());
        assert!(parse_page_range("0", 10).is_err());
        assert!(parse_page_range("5-2", 10).is_err());
    }
}
