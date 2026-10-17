// //// Neoffice — added file (no upstream equivalent): what the hosting page's answer to a renewal of tokens means for each
// mailbox waiting for it (backend.rs `renew`), apart from the page and the network so that it is tested natively.

//! When the server refuses a hosted mailbox's token, the backend asks the page for new ones (`host::renew`). Its answer
//! gives some mailboxes in full (with a secret), may name others without one yet, and leaves the rest out. Each mailbox
//! waiting for it then signs in again, asks again a little later, or asks for a reload; one started again since it asked
//! sees to itself.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::HashMap;

/// What the page answered.
#[derive(Clone, Debug, Default)]
pub struct Answer {
    /// The mailboxes it gives in full: address (lower case) → (secret, Bearer).
    pub given: HashMap<String, (String, bool)>,
    /// Every address it names (lower case), with a secret or not.
    pub named: Vec<String>,
}

/// What a mailbox waiting for the answer does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Sign in again with this secret.
    SignIn(String),
    /// Ask the page again a little later: it had nothing for the mailbox yet.
    AskLater,
    /// Only reloading the page gives it access again.
    Reload,
    /// Started again since it asked (or gone): its new start sees to itself.
    Skip,
}

/// The secrets kept after an answer: those of the mailboxes it gives in full replace theirs, and the others stay, so that
/// an answer naming a mailbox without a secret, or leaving it out, takes nothing from the other mailboxes (they stay
/// signed in the way the page said).
pub fn merge(host: &mut HashMap<String, (String, bool)>, answer: &Answer) {
    for (email, given) in &answer.given {
        host.insert(email.clone(), given.clone());
    }
}

/// What the mailbox `email` does after `answer` (`None`: the page gave none), asked for by its start `asked`, now at start
/// `now` (`None`: gone), having asked `tries` times in a row before, at most `max`.
pub fn next(answer: Option<&Answer>, email: &str, asked: u32, now: Option<u32>, tries: u8, max: u8) -> Next {
    if now != Some(asked) {
        return Next::Skip;
    }
    if let Some((secret, _)) = answer.and_then(|a| a.given.get(email)) {
        return Next::SignIn(secret.clone());
    }
    // Nothing for it yet: no answer, or one that names it without a secret.
    let later = answer.is_none_or(|a| a.named.iter().any(|n| n == email));
    if later && tries < max { Next::AskLater } else { Next::Reload }
}

/// The mailboxes waiting for an answer, each with the start that asked: one asking again (started again meanwhile)
/// waits with its new start.
pub fn wait(waiting: &mut Vec<(String, u32)>, id: &str, generation: u32) {
    match waiting.iter_mut().find(|(w, _)| w == id) {
        Some(entry) => entry.1 = generation,
        None => waiting.push((id.to_string(), generation)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(given: &[(&str, &str)], named: &[&str]) -> Answer {
        Answer {
            given: given.iter().map(|(e, s)| (e.to_string(), (s.to_string(), true))).collect(),
            named: named.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn an_answer_replaces_only_the_mailboxes_it_gives_in_full() {
        let mut host: HashMap<String, (String, bool)> =
            [("a@example.ch", "old-a"), ("b@example.ch", "old-b")].iter().map(|(e, s)| (e.to_string(), (s.to_string(), true))).collect();
        // Named without a secret, or left out: the other mailbox keeps its own, and stays signed in as a hosted one.
        merge(&mut host, &answer(&[("a@example.ch", "new-a")], &["a@example.ch", "b@example.ch"]));
        assert_eq!(host.get("a@example.ch").map(|(s, b)| (s.as_str(), *b)), Some(("new-a", true)));
        assert_eq!(host.get("b@example.ch").map(|(s, _)| s.as_str()), Some("old-b"));
        merge(&mut host, &answer(&[], &[]));
        assert_eq!(host.len(), 2, "an empty answer takes nothing away");
    }

    #[test]
    fn each_mailbox_waiting_signs_in_asks_later_reloads_or_sees_to_itself() {
        let given = answer(&[("a@example.ch", "new-a")], &["a@example.ch", "b@example.ch"]);
        assert_eq!(next(Some(&given), "a@example.ch", 1, Some(1), 0, 60), Next::SignIn("new-a".into()));
        assert_eq!(next(Some(&given), "b@example.ch", 1, Some(1), 0, 60), Next::AskLater, "named without a secret yet");
        assert_eq!(next(Some(&given), "c@example.ch", 1, Some(1), 0, 60), Next::Reload, "left out");
        assert_eq!(next(None, "c@example.ch", 1, Some(1), 59, 60), Next::AskLater, "no answer: again");
        assert_eq!(next(None, "c@example.ch", 1, Some(1), 60, 60), Next::Reload, "after an hour of them");
        // Started again (or removed) since it asked: its new start sees to itself, and nothing here holds it on
        // « Connecting ».
        assert_eq!(next(Some(&given), "a@example.ch", 1, Some(2), 0, 60), Next::Skip);
        assert_eq!(next(None, "a@example.ch", 1, None, 0, 60), Next::Skip);
        // A mailbox started again that asks while the first request is out waits with its new start.
        let mut waiting = Vec::new();
        wait(&mut waiting, "a1", 1);
        wait(&mut waiting, "a2", 1);
        wait(&mut waiting, "a1", 2);
        assert_eq!(waiting, vec![("a1".to_string(), 2), ("a2".to_string(), 1)]);
    }
}
