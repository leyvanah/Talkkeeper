//! Names the archive does not know yet, found by a model on this machine.
//!
//! The vocabulary holds the names the archive was told about: the library,
//! the people, the speakers, the owner's list. A name said in passing — "and
//! then Masha called" — is in none of them, and would leave as it is. Before a
//! conversation goes to a model off this machine, the built-in model reads the
//! words that could be names and says which are; those join the vocabulary
//! for that one request. Nothing here leaves the machine.
//!
//! The model does not read the whole conversation. Candidates are picked
//! cheaply first — capitalised words the text never writes in lower case —
//! and the model only judges those, each with a few words around it. Its
//! answer is trusted only as far as it repeats a candidate: a word it made up
//! is not in the text and cannot be hidden anyway.

use std::collections::HashSet;
use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use crate::summary::summary_engine::{client::generate_with_builtin, models};

/// Candidates the model judges in one prompt. Small enough for a 2B model to
/// keep track of every line of its answer.
const BATCH: usize = 40;
/// A long conversation can hold a great many capitalised words; past this the
/// rest are not judged. Logged, so the limit is never silent.
const MAX_CANDIDATES: usize = 400;
/// Words of context on each side of a candidate.
const CONTEXT_WORDS: usize = 5;

/// A word that could be someone's name, and where it was first seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub word: String,
    pub context: String,
}

fn words_of(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_alphabetic() || c == '-'))
        .map(|word| word.trim_matches('-'))
        .filter(|word| !word.is_empty())
        .collect()
}

/// Capitalised, then lower case, three letters at least: the hiding refuses
/// shorter names anyway ("Он", "Да" would only cost prompt space). An
/// all-capitals word is an abbreviation — an organisation, not a person.
fn looks_like_a_name(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let rest: Vec<char> = chars.collect();
    first.is_uppercase() && rest.len() >= 2 && rest.iter().any(|c| c.is_lowercase())
}

/// The words worth asking about, each once, in the order they first appear.
///
/// A word the text also writes in lower case is an ordinary word that began a
/// sentence. A word whose stem a known name already covers is hidden anyway.
pub fn candidates(text: &str, known: &[String]) -> Vec<Candidate> {
    let words = words_of(text);
    let lowered: HashSet<&str> = words
        .iter()
        .copied()
        .filter(|word| word.chars().next().is_some_and(char::is_lowercase))
        .collect();
    let known_stems: Vec<String> = known
        .iter()
        .map(|name| super::anonymize::stem_of(name.trim()).to_lowercase())
        .filter(|stem| stem.chars().count() >= 3)
        .collect();

    let mut seen: HashSet<String> = HashSet::new();
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        if !looks_like_a_name(word) {
            continue;
        }
        let lower = word.to_lowercase();
        if lowered.contains(lower.as_str()) {
            continue;
        }
        // Covered exactly when the hiding would catch it: the stem, then a
        // short ending. "Анна" covers "Анной", not "Анненков".
        let covered = |stem: &String| {
            lower.starts_with(stem.as_str())
                && lower.chars().count() - stem.chars().count() <= super::anonymize::MAX_TAIL
        };
        if known_stems.iter().any(covered) {
            continue;
        }
        if !seen.insert(super::anonymize::stem_of(&lower).to_string()) {
            continue;
        }
        let from = at.saturating_sub(CONTEXT_WORDS);
        let to = (at + CONTEXT_WORDS + 1).min(words.len());
        found.push(Candidate {
            word: word.to_string(),
            context: words[from..to].join(" "),
        });
    }
    found
}

const SYSTEM_PROMPT: &str = "Ты помогаешь убрать личные данные из текста. \
Отвечай только списком слов, без пояснений.";

fn user_prompt(batch: &[Candidate]) -> String {
    let mut prompt = String::from(
        "Ниже слова из расшифровки разговора, у каждого — кусочек текста вокруг.\n\
         Выпиши только те слова, которые здесь означают конкретного человека: имя, \
         фамилию, отчество или прозвище.\n\
         Не выписывай названия городов, стран, улиц, компаний, брендов, праздников \
         и обычные слова.\n\
         Каждое слово — на отдельной строке, ровно как оно дано в списке. \
         Если таких слов нет, напиши: нет\n\n",
    );
    for (index, candidate) in batch.iter().enumerate() {
        prompt.push_str(&format!(
            "{}. {} — «{}»\n",
            index + 1,
            candidate.word,
            candidate.context
        ));
    }
    prompt
}

/// The candidates the answer names, in the form the candidate list gave them.
pub fn parse_answer(answer: &str, batch: &[Candidate]) -> Vec<String> {
    let mut named = Vec::new();
    for line in answer.lines() {
        // "3. Маша — ..." or "- Маша": keep the first word-like run.
        let cleaned = line
            .trim()
            .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | '•'))
            .trim();
        let Some(word) = words_of(cleaned).into_iter().next() else {
            continue;
        };
        if let Some(candidate) = matching_candidate(word, batch) {
            if !named.contains(&candidate.word) {
                named.push(candidate.word.clone());
            }
        }
    }
    named
}

/// The candidate an answered word stands for.
///
/// Asked to copy the word, a model still tends to give the name in its
/// dictionary form: "Олег" for "Олегом", "Тимофей" for "Тимофея". Either the
/// answer begins the candidate, or the candidate's stem begins the answer —
/// never a looser likeness, so "Маша" does not claim "Машина".
fn matching_candidate<'a>(word: &str, batch: &'a [Candidate]) -> Option<&'a Candidate> {
    let answered = word.to_lowercase();
    if let Some(exact) = batch.iter().find(|c| c.word.to_lowercase() == answered) {
        return Some(exact);
    }
    if answered.chars().count() < 3 {
        return None;
    }
    batch.iter().find(|candidate| {
        let given = candidate.word.to_lowercase();
        let stem = super::anonymize::stem_of(&given);
        given.starts_with(&answered) || (stem.chars().count() >= 3 && answered.starts_with(stem))
    })
}

/// The best built-in model that is on disk, if any is.
pub fn local_model(app_data_dir: &PathBuf) -> Option<String> {
    models::get_available_models()
        .into_iter()
        .filter(|model| {
            models::get_model_path(app_data_dir, &model.name)
                .map(|path| path.exists())
                .unwrap_or(false)
        })
        .max_by_key(|model| crate::summary::summary_engine::commands::summary_model_priority(&model.name))
        .map(|model| model.name)
}

/// Names in `text` the vocabulary does not hold yet, as the local model sees
/// them. An error means the search could not run; the caller decides what
/// that costs.
pub async fn find(
    app_data_dir: &PathBuf,
    text: &str,
    known: &[String],
    cancellation_token: Option<&CancellationToken>,
) -> Result<Vec<String>, String> {
    let model = local_model(app_data_dir).ok_or("no built-in model is downloaded")?;
    let mut candidates = candidates(text, known);
    if candidates.len() > MAX_CANDIDATES {
        log::warn!(
            "🛡️ {} capitalised words in the text; only the first {} are judged",
            candidates.len(),
            MAX_CANDIDATES
        );
        candidates.truncate(MAX_CANDIDATES);
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for batch in candidates.chunks(BATCH) {
        let answer = generate_with_builtin(
            app_data_dir,
            &model,
            SYSTEM_PROMPT,
            &user_prompt(batch),
            cancellation_token,
        )
        .await
        .map_err(|error| error.to_string())?;
        names.extend(parse_answer(&answer, batch));
    }
    // Counts only: the words themselves are what must not reach the log.
    log::info!(
        "🛡️ Local model ({}) judged {} candidate words, {} of them names",
        model,
        candidates.len(),
        names.len()
    );
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(found: &[Candidate]) -> Vec<&str> {
        found.iter().map(|candidate| candidate.word.as_str()).collect()
    }

    #[test]
    fn a_capitalised_word_the_text_also_writes_small_is_not_asked_about() {
        let text = "Когда мы пришли, Маша уже ждала. А когда ушли — нет.";
        assert_eq!(words(&candidates(text, &[])), vec!["Маша"]);
    }

    #[test]
    fn a_name_the_archive_knows_is_left_to_the_vocabulary() {
        let text = "Вчера Анной было сказано, что Пётр опоздает.";
        assert_eq!(words(&candidates(text, &["Анна".to_string()])), vec!["Вчера", "Пётр"]);
        // A longer word that merely starts the same is not covered by it.
        let text = "Вчера звонил Анненков.";
        assert_eq!(words(&candidates(text, &["Анна".to_string()])), vec!["Вчера", "Анненков"]);
    }

    #[test]
    fn each_name_is_asked_about_once_whatever_its_form() {
        let text = "Маша пришла. Потом Маши не было. Я звонил Маше.";
        let found = candidates(text, &[]);
        // "Маши" and "Маше" share the stem; "Я" is too short to be asked about.
        assert_eq!(words(&found), vec!["Маша", "Потом"]);
        assert_eq!(found[0].context, "Маша пришла Потом Маши не было");
    }

    #[test]
    fn abbreviations_are_not_names() {
        let text = "Он работает в ВТБ, а она в МГУ.";
        assert!(candidates(text, &[]).is_empty());
    }

    #[test]
    fn the_answer_counts_only_where_it_repeats_a_candidate() {
        let batch = vec![
            Candidate { word: "Маша".into(), context: String::new() },
            Candidate { word: "Москва".into(), context: String::new() },
        ];
        let answer = "1. Маша\n- Серёжа\nнет\n";
        assert_eq!(parse_answer(answer, &batch), vec!["Маша"]);
        assert!(parse_answer("нет", &batch).is_empty());
    }

    #[test]
    fn the_answer_may_repeat_the_line_it_was_given() {
        let batch = vec![Candidate { word: "Маша".into(), context: "вчера Маша звонила".into() }];
        assert_eq!(parse_answer("1. Маша — «вчера Маша звонила»", &batch), vec!["Маша"]);
    }

    #[test]
    fn a_name_given_back_in_its_dictionary_form_still_counts() {
        let batch = vec![
            Candidate { word: "Олегом".into(), context: String::new() },
            Candidate { word: "Тимофея".into(), context: String::new() },
            Candidate { word: "Машина".into(), context: String::new() },
        ];
        // What a local model answered for a synthetic dialogue.
        assert_eq!(parse_answer("Олег\nТимофей", &batch), vec!["Олегом", "Тимофея"]);
        // A likeness is not enough: nothing in the batch is "Маша".
        assert!(parse_answer("Маша", &batch).is_empty());
    }

    #[test]
    fn the_prompt_lists_every_candidate_with_its_context() {
        let batch = vec![Candidate { word: "Маша".into(), context: "вчера Маша звонила".into() }];
        let prompt = user_prompt(&batch);
        assert!(prompt.contains("1. Маша — «вчера Маша звонила»"));
    }
}
