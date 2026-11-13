//! The nearest known key, for the error of a config file.

/// The nearest key of `known` (by edit distance), for an error.
pub(super) fn nearest_key(key: &str, known: &[&'static str]) -> Option<&'static str> {
    fn distance(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut prev = row[0];
            row[0] = i + 1;
            for (j, cb) in b.iter().enumerate() {
                let cur = row[j + 1];
                row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
                prev = cur;
            }
        }
        row[b.len()]
    }
    known
        .iter()
        .map(|k| (distance(key, k), *k))
        .min()
        // Issue 172: a short key is near each short key; "a" is not "tag".
        .filter(|(d, _)| *d <= (key.chars().count() / 3).max(1) && *d < key.chars().count())
        .map(|(_, k)| k)
}
