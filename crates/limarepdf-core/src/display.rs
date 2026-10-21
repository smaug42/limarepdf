//! Cleaning text from a file before it is shown, so a link, title or comment can't hide
//! what it is: bidi controls (which can make `moc.evil` read as `live.com`), invisible
//! characters and control characters are removed, and long text is shortened. Also the
//! app's own times, in local time.

/// Characters that change how text around them is ordered or that show nothing: bidi
/// embeddings, overrides, isolates and marks, plus zero-width spaces and the byte order
/// mark. The zero-width joiner stays, as emoji need it.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// `s` as one line for a label, title or dialog: invisible characters removed, line breaks
/// and other control characters turned into spaces (runs of them into one), trimmed, and
/// cut to `max` characters with "…" when longer.
pub fn display_line(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max.saturating_mul(4)));
    let mut space = false;
    for c in s.chars().filter(|&c| !invisible(c)) {
        if c.is_control() || c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    shorten(out, max)
}

/// `s` as text that may run over several lines, such as a comment: like [`display_line`],
/// but line breaks are kept (`\r\n` and `\r` become `\n`) and tabs stay.
pub fn display_text(s: &str, max: usize) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let out: String = s
        .chars()
        .filter(|&c| !invisible(c) && (!c.is_control() || c == '\n' || c == '\t'))
        .collect();
    shorten(out.trim().to_string(), max)
}

fn shorten(s: String, max: usize) -> String {
    match s.char_indices().nth(max.saturating_sub(1)) {
        Some((i, _)) if s[i..].chars().nth(1).is_some() => format!("{}…", s[..i].trim_end()),
        _ => s,
    }
}

/// `t` in this computer's time zone, as 2026-10-19 14:23 (the app's own times, such as when
/// a recovery copy was made).
pub fn local_time(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// The date of `t` in this computer's time zone, as 2026-10-19.
pub fn local_date(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bidi_and_controls() {
        assert_eq!(
            display_line("https://\u{202E}moc.live\u{202C}", 100),
            "https://moc.live"
        );
        assert_eq!(display_line("  a\n\r\tb\u{0}c  ", 100), "a b c");
        assert_eq!(display_line("x\u{200B}y\u{FEFF}", 100), "xy");
        assert_eq!(display_text("a\r\n\u{2067}b\u{7}\n\tc", 100), "a\nb\n\tc");
    }

    #[test]
    fn shortens() {
        assert_eq!(display_line("abcdef", 6), "abcdef");
        assert_eq!(display_line("abcdefg", 6), "abcde…");
        assert_eq!(display_line("ééééééé", 3), "éé…");
        assert_eq!(display_line("", 3), "");
    }
}
