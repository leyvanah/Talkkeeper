//! Replacing what identifies a person, before text is handed to a model that
//! is not on this machine.
//!
//! Only that path. A local model is inside the archive's own boundary and gets
//! the text as it is; so does anything the owner writes for themselves. This
//! exists for the one case where the owner deliberately sends a conversation
//! out of the house.
//!
//! Two properties matter more than cleverness here:
//!
//! * **It is reviewable.** Every replacement is reported, so a screen can show
//!   what will leave before anything leaves. Nothing is guessed at silently.
//! * **It is reversible on this machine.** The map from label back to the real
//!   word never goes anywhere: it stays in memory for the length of one
//!   request, so the answer that comes back can be put in the owner's own words
//!   again.
//!
//! What it cannot do is decide what identifies someone. A nickname, a street,
//! an unusual detail — those are the owner's to add, and the screen exists for
//! that. This module finds the names it was told about, in the forms Russian
//! grammar gives them, and the shapes that are mechanically recognisable.

use std::collections::HashMap;

/// What a replaced word was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Someone's name: the owner's, a speaker's, a person in the library.
    Name,
    /// A word the owner added: a place, an employer, anything identifying.
    Term,
    /// An e-mail address or a phone number.
    Contact,
    /// A link.
    Link,
    /// A long run of digits — an account, a card, a document.
    Number,
}

impl Kind {
    /// The word the label is built from. Russian: the interface is Russian
    /// first, and a Russian label reads naturally in a Russian transcript.
    fn label(self) -> &'static str {
        match self {
            Kind::Name => "Имя",
            Kind::Term => "Название",
            Kind::Contact => "Контакт",
            Kind::Link => "Ссылка",
            Kind::Number => "Номер",
        }
    }
}

/// One thing that was replaced, and what took its place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub kind: Kind,
    /// The label that went out, e.g. `[Имя 1]`.
    pub label: String,
    /// The word as it was written, in the first form it appeared in.
    pub original: String,
    /// How many times it was replaced, in every form.
    pub count: usize,
}

/// The result of a pass: the text that may leave, and what it cost.
#[derive(Debug, Clone, Default)]
pub struct Anonymized {
    pub text: String,
    pub replacements: Vec<Replacement>,
}

impl Anonymized {
    /// Puts the owner's own words back into an answer that came from outside.
    ///
    /// Labels a model repeated verbatim become the real words again; anything
    /// it invented is left alone, because there is nothing to map it to.
    pub fn restore(&self, text: &str) -> String {
        let mut restored = text.to_string();
        for replacement in &self.replacements {
            restored = restored.replace(&replacement.label, &replacement.original);
        }
        restored
    }
}

/// What the owner wants hidden, beyond the shapes this module recognises.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    /// Names: the owner's, the speakers', the people in the library.
    pub names: Vec<String>,
    /// Anything else the owner named: places, employers, uncommon words.
    pub terms: Vec<String>,
}

impl Vocabulary {
    fn entries(&self) -> Vec<(Kind, &str)> {
        let names = self.names.iter().map(|name| (Kind::Name, name.as_str()));
        let terms = self.terms.iter().map(|term| (Kind::Term, term.as_str()));
        names.chain(terms).collect()
    }
}

/// Russian inflects, so a name is rarely written the way it is stored:
/// "Анна" becomes "Анны", "Анне", "Анну". Matching the stem and letting a
/// short tail follow catches those without a dictionary of endings.
///
/// The stem is the word without its final vowel; a word ending in a consonant
/// keeps all of it. Short words keep all of it too — a two-letter stem would
/// match half the language.
fn stem_of(word: &str) -> &str {
    const VOWELS: [char; 12] = ['а', 'е', 'ё', 'и', 'й', 'о', 'у', 'ы', 'э', 'ю', 'я', 'ь'];
    let lowered: Vec<char> = word.chars().collect();
    if lowered.len() < 4 {
        return word;
    }
    let last = lowered[lowered.len() - 1].to_lowercase().next().unwrap_or(' ');
    if VOWELS.contains(&last) {
        let cut = word.len() - lowered[lowered.len() - 1].len_utf8();
        &word[..cut]
    } else {
        word
    }
}

/// The longest tail allowed after a stem, in characters. Russian endings are
/// short; three covers "Анной", "Марией", "Петровичем" is a separate word.
const MAX_TAIL: usize = 3;

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where `needle`'s stem matches in `haystack`, both already lowercased,
/// bounded by non-word characters, allowing a short inflected tail.
///
/// Returns byte ranges into `haystack`.
fn stem_matches(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let stem = stem_of(needle).to_lowercase();
    if stem.chars().count() < 3 {
        return Vec::new();
    }

    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = haystack[from..].find(&stem) {
        let start = from + at;
        let end = start + stem.len();
        from = end;

        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .map_or(true, |c| !is_word_char(c));
        if !before_ok {
            continue;
        }

        // Take up to MAX_TAIL letters of inflection, stopping at anything that
        // is not a letter.
        let mut tail_end = end;
        for c in haystack[end..].chars().take(MAX_TAIL) {
            if !c.is_alphabetic() {
                break;
            }
            tail_end += c.len_utf8();
        }
        let after_ok = haystack[tail_end..]
            .chars()
            .next()
            .map_or(true, |c| !is_word_char(c));
        if !after_ok {
            continue;
        }

        found.push((start, tail_end));
        from = tail_end;
    }
    found
}

/// The shapes that need no dictionary: contacts, links, long numbers.
fn pattern_matches(text: &str) -> Vec<(usize, usize, Kind)> {
    let bytes = text.as_bytes();
    let mut found: Vec<(usize, usize, Kind)> = Vec::new();

    // Links first: an address can hold an @ and digits, and the longest match
    // is the honest one.
    for prefix in ["https://", "http://", "www."] {
        let mut from = 0;
        while let Some(at) = text[from..].find(prefix) {
            let start = from + at;
            let mut end = start;
            for c in text[start..].chars() {
                if c.is_whitespace() || c == '"' || c == '<' || c == '>' {
                    break;
                }
                end += c.len_utf8();
            }
            // Trailing punctuation belongs to the sentence, not the address.
            while end > start && matches!(bytes[end - 1], b'.' | b',' | b')' | b';' | b':' | b'!' | b'?') {
                end -= 1;
            }
            found.push((start, end, Kind::Link));
            from = end.max(start + prefix.len());
        }
    }

    // An e-mail: word characters around an @ with a dot after it.
    let mut from = 0;
    while let Some(at) = text[from..].find('@') {
        let at = from + at;
        from = at + 1;
        let start = text[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word_char(*c) || *c == '.' || *c == '-' || *c == '+')
            .last()
            .map(|(index, _)| index)
            .unwrap_or(at);
        let mut end = at + 1;
        for c in text[at + 1..].chars() {
            if !(is_word_char(c) || c == '.' || c == '-') {
                break;
            }
            end += c.len_utf8();
        }
        while end > at && text[..end].ends_with('.') {
            end -= 1;
        }
        if start < at && text[at..end].contains('.') {
            found.push((start, end, Kind::Contact));
        }
    }

    // Phone numbers and long digit runs. A run of digits with the usual
    // separators counts as a phone when it is long enough to be one.
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        let (start, c) = chars[index];
        if !(c.is_ascii_digit() || c == '+') {
            index += 1;
            continue;
        }
        let mut end = start + c.len_utf8();
        let mut digits = usize::from(c.is_ascii_digit());
        let mut cursor = index + 1;
        while cursor < chars.len() {
            let (offset, next) = chars[cursor];
            if next.is_ascii_digit() {
                digits += 1;
            } else if !matches!(next, ' ' | '-' | '(' | ')') {
                break;
            } else if digits == 0 {
                break;
            }
            end = offset + next.len_utf8();
            cursor += 1;
        }
        while end > start && text[..end].ends_with([' ', '-', '(', ')']) {
            end -= 1;
        }
        if digits >= 7 {
            let kind = if text[start..end].contains(['+', '-', '(', ' ']) {
                Kind::Contact
            } else {
                Kind::Number
            };
            found.push((start, end, kind));
        }
        index = cursor.max(index + 1);
    }

    found
}

/// One request's worth of replacing, so that every piece of text going to the
/// same place speaks the same language.
///
/// A request is usually more than one string — an instruction and a
/// conversation, sometimes a chunk at a time. Replacing them separately would
/// give the same person a different label in each, and the model would read
/// them as different people. A session carries the labels across all of them,
/// and back again over the answer.
#[derive(Debug, Clone)]
pub struct Session {
    vocabulary: Vocabulary,
    labels: HashMap<(Kind, String), String>,
    counts: HashMap<String, usize>,
    order: Vec<(Kind, String, String)>,
    next_number: HashMap<Kind, usize>,
}

impl Session {
    pub fn new(vocabulary: Vocabulary) -> Self {
        Self {
            vocabulary,
            labels: HashMap::new(),
            counts: HashMap::new(),
            order: Vec::new(),
            next_number: HashMap::new(),
        }
    }

    /// The text as it may leave, with everything recognised replaced.
    pub fn hide(&mut self, text: &str) -> String {
        hide_into(self, text)
    }

    /// Everything replaced so far, in the order it was first met.
    pub fn replacements(&self) -> Vec<Replacement> {
        self.order
            .iter()
            .map(|(kind, label, original)| Replacement {
                kind: *kind,
                count: self.counts.get(label).copied().unwrap_or(0),
                label: label.clone(),
                original: original.clone(),
            })
            .collect()
    }

    /// Puts the owner's own words back into an answer that came from outside.
    pub fn restore(&self, text: &str) -> String {
        let mut restored = text.to_string();
        for (_, label, original) in &self.order {
            restored = restored.replace(label, original);
        }
        restored
    }

    /// Whether anything was actually replaced.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Replaces what the vocabulary names, and what the patterns recognise.
///
/// Overlapping finds are resolved in favour of the longer one, so a name
/// inside an address does not cut it in half.
pub fn anonymize(text: &str, vocabulary: &Vocabulary) -> Anonymized {
    let mut session = Session::new(vocabulary.clone());
    let text = session.hide(text);
    Anonymized {
        text,
        replacements: session.replacements(),
    }
}

fn hide_into(session: &mut Session, text: &str) -> String {
    let vocabulary = &session.vocabulary;
    let lowered = text.to_lowercase();
    // Lowercasing can change byte lengths (rare in Russian, real in Turkish);
    // if it does, the offsets below would not line up, so fall back to
    // patterns only rather than cut a word in the middle.
    let offsets_align = lowered.len() == text.len();

    let mut spans: Vec<(usize, usize, Kind, Option<usize>)> = Vec::new();
    if offsets_align {
        for (index, (kind, word)) in vocabulary.entries().iter().enumerate() {
            for (start, end) in stem_matches(&lowered, word) {
                spans.push((start, end, *kind, Some(index)));
            }
        }
    }
    for (start, end, kind) in pattern_matches(text) {
        spans.push((start, end, kind, None));
    }

    // Longest first, so the winner of an overlap is the more complete reading.
    spans.sort_by(|a, b| a.0.cmp(&b.0).then((b.1 - b.0).cmp(&(a.1 - a.0))));
    let mut kept: Vec<(usize, usize, Kind, Option<usize>)> = Vec::new();
    for span in spans {
        if kept.last().is_some_and(|last| span.0 < last.1) {
            continue;
        }
        kept.push(span);
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;

    for (start, end, kind, entry) in kept {
        out.push_str(&text[cursor..start]);
        let written = &text[start..end];
        // One key per thing: a vocabulary word keeps its identity across the
        // forms it takes, while a pattern is keyed by what it matched.
        let key = match entry {
            Some(index) => format!("#{index}"),
            None => written.to_lowercase(),
        };
        let label = match session.labels.get(&(kind, key.clone())) {
            Some(label) => label.clone(),
            None => {
                let number = session.next_number.entry(kind).or_insert(0);
                *number += 1;
                let label = format!("[{} {}]", kind.label(), number);
                session.order.push((kind, label.clone(), written.to_string()));
                session.labels.insert((kind, key), label.clone());
                label
            }
        };
        *session.counts.entry(label.clone()).or_insert(0) += 1;
        out.push_str(&label);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocabulary(names: &[&str], terms: &[&str]) -> Vocabulary {
        Vocabulary {
            names: names.iter().map(|s| s.to_string()).collect(),
            terms: terms.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_name_is_replaced_in_every_form_russian_gives_it() {
        let text = "Анна пришла. Я сказал Анне, что Анны не будет, и ждал Анну.";
        let result = anonymize(text, &vocabulary(&["Анна"], &[]));

        assert!(!result.text.contains("Анн"), "{}", result.text);
        assert_eq!(result.text.matches("[Имя 1]").count(), 4);
        assert_eq!(result.replacements.len(), 1);
        assert_eq!(result.replacements[0].count, 4);
        assert_eq!(result.replacements[0].original, "Анна");
    }

    #[test]
    fn the_same_person_keeps_one_label_and_others_get_their_own() {
        let text = "Анна и Пётр. Пётр ушёл, Анна осталась.";
        let result = anonymize(text, &vocabulary(&["Анна", "Пётр"], &[]));

        assert_eq!(result.text, "[Имя 1] и [Имя 2]. [Имя 2] ушёл, [Имя 1] осталась.");
    }

    #[test]
    fn a_word_that_merely_starts_the_same_is_left_alone() {
        // A tail longer than an ending means a different word: "Аннушка" is
        // not "Анна" inflected. That is the trade this makes — it would rather
        // leave a diminutive standing than replace half the language — and it
        // is why the owner reviews the list and can add the diminutive itself.
        let text = "Аннушка разлила масло, а банан остался.";
        let result = anonymize(text, &vocabulary(&["Анна"], &[]));

        assert_eq!(result.text, text);
        assert!(result.replacements.is_empty());
    }

    #[test]
    fn a_diminutive_the_owner_added_is_caught_like_any_other_name() {
        let text = "Аннушка разлила масло.";
        let result = anonymize(text, &vocabulary(&["Анна", "Аннушка"], &[]));

        // Numbered by where they appear in the text, not by where they sit
        // in the vocabulary.
        assert_eq!(result.text, "[Имя 1] разлила масло.");
    }

    #[test]
    fn contacts_links_and_long_numbers_need_no_dictionary() {
        let text = "Пишите на anna.smith@example.com или звоните +7 999 123-45-67. \
                    Счёт 40817810099910004312, сайт https://example.com/page.";
        let result = anonymize(text, &Vocabulary::default());

        assert!(!result.text.contains('@'), "{}", result.text);
        assert!(!result.text.contains("999"), "{}", result.text);
        assert!(!result.text.contains("40817810099910004312"), "{}", result.text);
        assert!(!result.text.contains("example.com/page"), "{}", result.text);
        assert!(result.text.contains("[Контакт 1]"));
        assert!(result.text.contains("[Номер 1]"));
        assert!(result.text.contains("[Ссылка 1]"));
    }

    #[test]
    fn a_short_number_is_not_mistaken_for_a_phone() {
        let text = "Мы говорили 45 минут, в 2026 году, по 3 раза в неделю.";
        let result = anonymize(text, &Vocabulary::default());

        assert_eq!(result.text, text);
        assert!(result.replacements.is_empty());
    }

    #[test]
    fn an_answer_comes_back_in_the_owners_own_words() {
        let text = "Анна написала на anna@example.com.";
        let result = anonymize(text, &vocabulary(&["Анна"], &[]));
        let from_the_model = format!(
            "{} упоминает {} и адрес {}.",
            result.replacements[0].label, result.replacements[0].label, result.replacements[1].label
        );

        let restored = result.restore(&from_the_model);

        assert!(restored.contains("Анна"));
        assert!(restored.contains("anna@example.com"));
        assert!(!restored.contains('['));
    }

    #[test]
    fn a_term_the_owner_added_is_replaced_like_a_name() {
        let text = "Встречаемся в Простоквашино, недалеко от Простоквашина.";
        let result = anonymize(text, &vocabulary(&[], &["Простоквашино"]));

        assert_eq!(result.text, "Встречаемся в [Название 1], недалеко от [Название 1].");
    }

    #[test]
    fn one_request_speaks_one_language_across_all_its_pieces() {
        // An instruction and a conversation go out together; the same person
        // must be the same person in both, or the model reads two people.
        let mut session = Session::new(vocabulary(&["Анна", "Пётр"], &[]));

        let instruction = session.hide("Сведи разговор. Пётр — ведущий.");
        let conversation = session.hide("Анна: здравствуйте. Пётр: добрый день.");

        assert_eq!(instruction, "Сведи разговор. [Имя 1] — ведущий.");
        assert_eq!(conversation, "[Имя 2]: здравствуйте. [Имя 1]: добрый день.");
        assert_eq!(session.replacements().len(), 2);
        assert_eq!(session.restore("[Имя 1] и [Имя 2]"), "Пётр и Анна");
    }

    #[test]
    fn nothing_is_replaced_when_there_is_nothing_to_replace() {
        let text = "Мы обсудили планы на неделю.";
        let result = anonymize(text, &vocabulary(&["Анна"], &[]));

        assert_eq!(result.text, text);
        assert!(result.replacements.is_empty());
    }

    #[test]
    fn a_two_letter_name_is_refused_rather_than_shredding_the_text() {
        // A stem shorter than three characters would match everywhere.
        let text = "Я и он говорили об этом.";
        let result = anonymize(text, &vocabulary(&["Ян"], &[]));

        assert_eq!(result.text, text);
    }
}
