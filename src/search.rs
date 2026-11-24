//! Fuzzy matching for the search box. No allocations on the hot path beyond lowercasing.

/// Score `query` against `text`; `None` = no match. Higher is better.
/// Prefers: exact > prefix > word-prefix > substring > in-order subsequence (with bonuses for
/// matches at word starts and consecutive runs). Case-insensitive.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t_lower = text.to_lowercase();
    let q_str: String = query.trim().to_lowercase();
    if t_lower == q_str {
        return Some(10_000);
    }
    if t_lower.starts_with(&q_str) {
        return Some(8_000 - t_lower.len() as i32);
    }
    // Word prefix ("ring" in "elden ring")
    if t_lower.split(|c: char| !c.is_alphanumeric()).any(|w| !w.is_empty() && w.starts_with(&q_str)) {
        return Some(6_000 - t_lower.len() as i32);
    }
    if t_lower.contains(&q_str) {
        return Some(4_000 - t_lower.len() as i32);
    }
    // Acronym / subsequence, ignoring spaces in the query. Word starts are taken from the original
    // text so camel-case humps count too ("osrs" → Old School RuneScape).
    let mut t: Vec<char> = Vec::with_capacity(text.len());
    let mut starts: Vec<bool> = Vec::with_capacity(text.len());
    let mut prev: Option<char> = None;
    for c in text.chars() {
        let word_start = match prev {
            None => true,
            Some(p) => {
                !p.is_alphanumeric()
                    || (c.is_ascii_digit() && !p.is_ascii_digit())
                    || (c.is_uppercase() && p.is_lowercase())
            }
        };
        for (k, l) in c.to_lowercase().enumerate() {
            t.push(l);
            starts.push(word_start && k == 0);
        }
        prev = Some(c);
    }
    let mut qi = 0;
    let mut s = 0i32;
    let mut prev_match: Option<usize> = None;
    for (ti, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            // Loose matches must stay meaningful: every matched char either starts a word
            // ("ff7" → Final Fantasy 7) or continues the previous match ("witch" in "witcher").
            let word_start = starts[ti];
            let consecutive = prev_match == Some(ti.wrapping_sub(1));
            if !word_start && !consecutive {
                continue;
            }
            s += 10;
            if word_start {
                s += 30;
            }
            if consecutive {
                s += 15;
            }
            prev_match = Some(ti);
            qi += 1;
        }
    }
    if qi == q.len() { Some(s - t.len() as i32) } else { None }
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn ranking() {
        let exact = score("doom", "DOOM").unwrap();
        let prefix = score("doom", "DOOM Eternal").unwrap();
        let word = score("ring", "ELDEN RING").unwrap();
        let sub = score("lden", "ELDEN RING").unwrap();
        let acr = score("ff7", "FINAL FANTASY VII 7").unwrap();
        assert!(exact > prefix && prefix > word && word > sub && sub > acr);
        assert!(score("xyz", "ELDEN RING").is_none());
        assert!(score("er", "ELDEN RING").is_some());
        assert!(score("wild hunt", "The Witcher 3 Wild Hunt").is_some());
        // Too-loose subsequences are rejected.
        assert!(score("dead", "Shadow of the Tomb Raider").is_none());
        assert!(score("dead", "Pathfinder Wrath of the Righteous").is_none());
        assert!(score("dead", "Dead Island Definitive Edition").is_some());
        assert!(score("tlou", "The Last of Us").is_some());
        assert!(score("bg3", "Baldur's Gate 3").is_some());
        assert!(score("osrs", "Old School RuneScape").is_some());
        assert!(score("mr", "Metaphor: ReFantazio").is_some());
        assert!(score("cape", "Old School RuneScape").is_some()); // substring
        assert!(score("İs", "İstanbul Stories").is_some()); // lowercase expands to two chars
    }
}
