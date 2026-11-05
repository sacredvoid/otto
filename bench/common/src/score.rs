use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Speech,
    Silence,
}

#[derive(Debug, Clone)]
pub struct Command {
    pub id: String,
    pub text: String,
    pub entity: String,
    pub kind: Kind,
}

/// Load commands from a TSV with columns: id, text, entity, kind.
/// A leading header row (first column `id`) and blank lines are skipped.
pub fn load_commands(tsv: &Path) -> Result<Vec<Command>, String> {
    let raw = std::fs::read_to_string(tsv).map_err(|e| format!("read {}: {e}", tsv.display()))?;
    let mut out = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if i == 0 && cols[0].trim().eq_ignore_ascii_case("id") {
            continue;
        }
        if cols.len() < 4 {
            return Err(format!("line {}: expected 4 tab-separated columns", i + 1));
        }
        let kind = match cols[3].trim().to_lowercase().as_str() {
            "speech" => Kind::Speech,
            "silence" => Kind::Silence,
            other => return Err(format!("line {}: unknown kind '{other}'", i + 1)),
        };
        out.push(Command {
            id: cols[0].trim().to_string(),
            text: cols[1].trim().to_string(),
            entity: cols[2].trim().to_string(),
            kind,
        });
    }
    Ok(out)
}

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

fn number_words(mut n: u32, out: &mut Vec<String>) {
    if n >= 1000 {
        out.push(ONES[(n / 1000) as usize].into());
        out.push("thousand".into());
        n %= 1000;
        if n == 0 {
            return;
        }
    }
    if n >= 100 {
        out.push(ONES[(n / 100) as usize].into());
        out.push("hundred".into());
        n %= 100;
        if n == 0 {
            return;
        }
    }
    if n >= 20 {
        out.push(TENS[(n / 10) as usize].into());
        if n % 10 != 0 {
            out.push(ONES[(n % 10) as usize].into());
        }
    } else {
        out.push(ONES[n as usize].into());
    }
}

/// Lowercase, strip punctuation, spell digits (0..=9999) and email "@"/"." as words.
/// "4:15", "4.15" and "four fifteen" normalize identically.
pub fn normalize(s: &str) -> String {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    let mut spaced = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        match c {
            '@' => spaced.push_str(" at "),
            '.' => {
                let p = prev.is_some_and(|x| x.is_alphanumeric());
                let n = next.is_some_and(|x| x.is_alphanumeric());
                let digits = prev.is_some_and(|x| x.is_ascii_digit())
                    && next.is_some_and(|x| x.is_ascii_digit());
                if p && n && !digits {
                    spaced.push_str(" dot ");
                } else {
                    spaced.push(' ');
                }
            }
            '\'' | '\u{2019}' => {}
            c if c.is_alphanumeric() => spaced.push(c),
            _ => spaced.push(' '),
        }
    }
    let mut words: Vec<String> = Vec::new();
    for tok in spaced.split_whitespace() {
        match tok.parse::<u32>() {
            Ok(n) if tok.chars().all(|c| c.is_ascii_digit()) && n <= 9999 => {
                number_words(n, &mut words)
            }
            _ => words.push(tok.to_string()),
        }
    }
    words.join(" ")
}

/// Word-level Levenshtein distance / reference length, on normalized text.
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    let r_n = normalize(reference);
    let h_n = normalize(hypothesis);
    let r: Vec<&str> = r_n.split_whitespace().collect();
    let h: Vec<&str> = h_n.split_whitespace().collect();
    if r.is_empty() {
        return h.len() as f64;
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            let sub = prev[j - 1] + usize::from(r[i - 1] != h[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}

/// True when the normalized entity appears as a whole-word phrase in the normalized hypothesis.
pub fn entity_hit(entity: &str, hypothesis: &str) -> bool {
    let e = normalize(entity);
    if e.is_empty() {
        return false;
    }
    format!(" {} ", normalize(hypothesis)).contains(&format!(" {e} "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_cases() {
        assert_eq!(wer("open notion", "open notion"), 0.0);
        assert_eq!(wer("email priya the summary", "email maya the summary"), 0.25);
    }

    #[test]
    fn entity_cases() {
        assert!(entity_hit("4:15", "remind me at four fifteen"));
        assert!(entity_hit("4:15", "remind me at 4.15"));
        assert!(!entity_hit("priya", "email maya"));
        assert!(entity_hit("john@acme.com", "send it to john at acme dot com"));
        assert_ne!(normalize("four hundred and fifteen"), normalize("four fifteen"));
        assert_eq!(normalize("4:15"), normalize("four fifteen"));
        assert_eq!(normalize("4.15"), normalize("four fifteen"));
    }
}
