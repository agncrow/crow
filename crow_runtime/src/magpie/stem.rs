//! The Porter stemmer (Porter, 1980), for English, as a log that
//! declares `stem` applies it to every token at the index and at the
//! query: `running`, `runs` and `runner` all `run`. The algorithm's
//! five steps, written out as the paper gives them, over ASCII letters;
//! a token with any other character is left as it is.

fn is_consonant(w: &[u8], i: usize) -> bool {
    match w[i] {
        b'a' | b'e' | b'i' | b'o' | b'u' => false,
        b'y' => i == 0 || !is_consonant(w, i - 1),
        _ => true,
    }
}

/// The measure of `w[..=end]`: how many times a run of vowels is
/// followed by a run of consonants.
fn measure(w: &[u8], end: usize) -> usize {
    let mut m = 0;
    let mut i = 0;
    let n = end;
    while i < n && is_consonant(w, i) {
        i += 1;
    }
    loop {
        while i < n && !is_consonant(w, i) {
            i += 1;
        }
        if i >= n {
            return m;
        }
        m += 1;
        while i < n && is_consonant(w, i) {
            i += 1;
        }
        if i >= n {
            return m;
        }
    }
}

fn has_vowel(w: &[u8], end: usize) -> bool {
    (0..end).any(|i| !is_consonant(w, i))
}

fn double_consonant(w: &[u8]) -> bool {
    let n = w.len();
    n >= 2 && w[n - 1] == w[n - 2] && is_consonant(w, n - 1)
}

/// Consonant, vowel, consonant at the end, the last not w, x or y.
fn cvc(w: &[u8]) -> bool {
    let n = w.len();
    n >= 3 && is_consonant(w, n - 1) && !is_consonant(w, n - 2) && is_consonant(w, n - 3) && !matches!(w[n - 1], b'w' | b'x' | b'y')
}

fn ends(w: &[u8], suffix: &str) -> bool {
    w.len() >= suffix.len() && &w[w.len() - suffix.len()..] == suffix.as_bytes()
}

/// `suffix` replaced by `with` where the stem before it has measure past `m`.
fn replace_if(w: &mut Vec<u8>, suffix: &str, with: &str, m: usize) -> bool {
    if !ends(w, suffix) {
        return false;
    }
    let stem = w.len() - suffix.len();
    if measure(w, stem) > m {
        w.truncate(stem);
        w.extend_from_slice(with.as_bytes());
    }
    true
}

pub fn stem(word: &str) -> String {
    if !word.bytes().all(|b| b.is_ascii_lowercase()) || word.len() <= 2 {
        return word.to_string();
    }
    let mut w: Vec<u8> = word.as_bytes().to_vec();
    // step 1a
    // (sses → ss and ies → i both lose two letters)
    if ends(&w, "sses") || ends(&w, "ies") {
        w.truncate(w.len() - 2);
    } else if ends(&w, "ss") {
    } else if ends(&w, "s") {
        w.pop();
    }
    // step 1b
    let mut more = false;
    if ends(&w, "eed") {
        if measure(&w, w.len() - 3) > 0 {
            w.pop();
        }
    } else if ends(&w, "ed") && has_vowel(&w, w.len() - 2) {
        w.truncate(w.len() - 2);
        more = true;
    } else if ends(&w, "ing") && has_vowel(&w, w.len() - 3) {
        w.truncate(w.len() - 3);
        more = true;
    }
    if more {
        if ends(&w, "at") || ends(&w, "bl") || ends(&w, "iz") {
            w.push(b'e');
        } else if double_consonant(&w) && !matches!(w[w.len() - 1], b'l' | b's' | b'z') {
            w.pop();
        } else if measure(&w, w.len()) == 1 && cvc(&w) {
            w.push(b'e');
        }
    }
    // step 1c
    if ends(&w, "y") && has_vowel(&w, w.len() - 1) {
        let n = w.len();
        w[n - 1] = b'i';
    }
    // step 2
    for (s, r) in [("ational", "ate"), ("tional", "tion"), ("enci", "ence"), ("anci", "ance"), ("izer", "ize"), ("abli", "able"), ("alli", "al"), ("entli", "ent"), ("eli", "e"), ("ousli", "ous"), ("ization", "ize"), ("ation", "ate"), ("ator", "ate"), ("alism", "al"), ("iveness", "ive"), ("fulness", "ful"), ("ousness", "ous"), ("aliti", "al"), ("iviti", "ive"), ("biliti", "ble")] {
        if replace_if(&mut w, s, r, 0) {
            break;
        }
    }
    // step 3
    for (s, r) in [("icate", "ic"), ("ative", ""), ("alize", "al"), ("iciti", "ic"), ("ical", "ic"), ("ful", ""), ("ness", "")] {
        if replace_if(&mut w, s, r, 0) {
            break;
        }
    }
    // step 4
    let mut done = false;
    for s in ["al", "ance", "ence", "er", "ic", "able", "ible", "ant", "ement", "ment", "ent", "ou", "ism", "ate", "iti", "ous", "ive", "ize"] {
        if ends(&w, s) {
            let stem_end = w.len() - s.len();
            if measure(&w, stem_end) > 1 {
                w.truncate(stem_end);
            }
            done = true;
            break;
        }
    }
    if !done && ends(&w, "ion") {
        let stem_end = w.len() - 3;
        if stem_end > 0 && measure(&w, stem_end) > 1 && matches!(w[stem_end - 1], b's' | b't') {
            w.truncate(stem_end);
        }
    }
    // step 5a
    if ends(&w, "e") {
        let stem_end = w.len() - 1;
        let m = measure(&w, stem_end);
        if m > 1 || (m == 1 && !cvc(&w[..stem_end])) {
            w.pop();
        }
    }
    // step 5b
    if measure(&w, w.len()) > 1 && double_consonant(&w) && w[w.len() - 1] == b'l' {
        w.pop();
    }
    String::from_utf8(w).unwrap_or_else(|_| word.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porters_examples_stem_as_the_paper_says() {
        for (word, want) in [
            ("caresses", "caress"), ("ponies", "poni"), ("ties", "ti"), ("caress", "caress"), ("cats", "cat"),
            ("feed", "feed"), ("agreed", "agre"), ("plastered", "plaster"), ("bled", "bled"), ("motoring", "motor"), ("sing", "sing"),
            ("conflated", "conflat"), ("troubled", "troubl"), ("sized", "size"), ("hopping", "hop"), ("tanned", "tan"), ("falling", "fall"), ("hissing", "hiss"), ("fizzed", "fizz"), ("failing", "fail"), ("filing", "file"),
            ("happy", "happi"), ("sky", "sky"),
            ("relational", "relat"), ("conditional", "condit"), ("rational", "ration"), ("valenci", "valenc"), ("hesitanci", "hesit"), ("digitizer", "digit"), ("conformabli", "conform"), ("radicalli", "radic"), ("differentli", "differ"), ("vileli", "vile"), ("analogousli", "analog"), ("vietnamization", "vietnam"), ("predication", "predic"), ("operator", "oper"), ("feudalism", "feudal"), ("decisiveness", "decis"), ("hopefulness", "hope"), ("callousness", "callous"), ("formaliti", "formal"), ("sensitiviti", "sensit"), ("sensibiliti", "sensibl"),
            ("triplicate", "triplic"), ("formative", "form"), ("formalize", "formal"), ("electriciti", "electr"), ("electrical", "electr"), ("hopeful", "hope"), ("goodness", "good"),
            ("revival", "reviv"), ("allowance", "allow"), ("inference", "infer"), ("airliner", "airlin"), ("gyroscopic", "gyroscop"), ("adjustable", "adjust"), ("defensible", "defens"), ("irritant", "irrit"), ("replacement", "replac"), ("adjustment", "adjust"), ("dependent", "depend"), ("adoption", "adopt"), ("homologou", "homolog"), ("communism", "commun"), ("activate", "activ"), ("angulariti", "angular"), ("homologous", "homolog"), ("effective", "effect"), ("bowdlerize", "bowdler"),
            ("probate", "probat"), ("rate", "rate"), ("cease", "ceas"), ("controll", "control"), ("roll", "roll"),
            ("running", "run"), ("generalization", "gener"), ("the", "the"), ("a", "a"), ("Fox", "Fox"), ("2026", "2026"),
        ] {
            assert_eq!(stem(word), want, "stem of {}", word);
        }
    }
}
