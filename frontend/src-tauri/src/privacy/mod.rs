//! Keeping what identifies a person out of what leaves the machine.

pub mod anonymize;
pub mod commands;
pub mod settings;
pub mod vocabulary;

use std::sync::Mutex;

use anonymize::Session;

/// One request's replacements, shared by everything that request touches.
///
/// A summary is rarely one call: a long conversation goes out in chunks, and
/// the chunk summaries come back for a final pass. They all have to name the
/// same person the same way, so they share one session. `None` means nothing
/// is hidden — the owner turned it off, or the destination is this machine.
pub type Shield<'a> = Option<&'a Mutex<Session>>;

fn with_session<T>(shield: Shield, action: impl FnOnce(&mut Session) -> T) -> Option<T> {
    let mutex = shield?;
    let mut session = mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    Some(action(&mut session))
}

/// The text as it may leave, or the text unchanged when nothing is hidden.
pub fn hide(shield: Shield, text: &str) -> String {
    with_session(shield, |session| session.hide(text)).unwrap_or_else(|| text.to_string())
}

/// The owner's own words back in an answer that came from outside.
pub fn restore(shield: Shield, text: &str) -> String {
    with_session(shield, |session| session.restore(text)).unwrap_or_else(|| text.to_string())
}

/// How many things this request hid, for the log and for the window.
pub fn hidden_count(shield: Shield) -> usize {
    with_session(shield, |session| session.replacements().len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anonymize::Vocabulary;

    fn session() -> Mutex<Session> {
        Mutex::new(Session::new(Vocabulary {
            names: vec!["Анна".to_string()],
            terms: Vec::new(),
        }))
    }

    #[test]
    fn without_a_shield_the_text_goes_as_it_is() {
        let text = "Анна пришла.";
        assert_eq!(hide(None, text), text);
        assert_eq!(restore(None, text), text);
        assert_eq!(hidden_count(None), 0);
    }

    #[test]
    fn with_a_shield_the_name_is_hidden_and_comes_back() {
        let shield = session();
        let sent = hide(Some(&shield), "Анна пришла.");

        assert_eq!(sent, "[Имя 1] пришла.");
        assert_eq!(hidden_count(Some(&shield)), 1);
        assert_eq!(restore(Some(&shield), "[Имя 1] опоздала"), "Анна опоздала");
    }
}
