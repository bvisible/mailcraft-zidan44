// //// Neoffice — added file (no upstream equivalent): the engine's and the network layers' messages in the interface
// language (toasts, a mailbox's status).

//! The engine and the network layers write their messages in English, some with values in them (an address, a server's
//! reason). A message the catalogs have whole is translated as it is; the known shapes of the others (a change the server
//! refused, a sign-in refused, a mailbox that needs its password, the hosting page's answers) are recognised, translated,
//! and their values put back, themselves translated when they are known messages too. Anything else shows as written.

/// `text` in the interface language, as far as its shape is known.
pub fn translate(text: &str) -> String {
    let whole = tl!(text);
    if whole != text {
        return whole.to_string();
    }
    let t = translate;
    // A change, or a sync of the calendars and contacts, the server refused, and why.
    if let Some(error) = text.strip_prefix("The server refused a change: ") {
        return tf!("The server refused a change: {error}", error = t(error));
    }
    if let Some(error) = text.strip_prefix("calendars and contacts: ") {
        return tf!("calendars and contacts: {error}", error = t(error));
    }
    if let Some(reason) = text.strip_prefix("a contact was not deleted: ") {
        return tf!("a contact was not deleted: {reason}", reason = t(reason));
    }
    if let Some(contact) = text.strip_suffix(": its address book can't be changed any more, so the change made here was undone") {
        return tf!("{contact}: its address book can't be changed any more, so the change made here was undone", contact = contact);
    }
    // What the network layers say went wrong (the JMAP transport's kinds of error).
    if let Some(reason) = text.strip_prefix("sign-in failed: ") {
        return tf!("sign-in failed: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("network: ") {
        return tf!("network: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("secure connection: ") {
        return tf!("secure connection: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("unexpected server response: ") {
        return tf!("unexpected server response: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("the server refused: ") {
        return tf!("the server refused: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("the server rejected the command: ") {
        return tf!("the server rejected the command: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("the server closed the connection: ") {
        return tf!("the server closed the connection: {reason}", reason = t(reason));
    }
    if let Some(reason) = text.strip_prefix("sending failed: ") {
        return tf!("sending failed: {reason}", reason = t(reason));
    }
    // A mailbox's: « {account} needs its password[: why] », « {account} needs you to sign in again[: why] ».
    if let Some((account, rest)) = text.split_once(' ').filter(|(a, _)| a.contains('@')) {
        if let Some(why) = rest.strip_prefix("needs its password: ") {
            return tf!("{account} needs its password: {reason}", account = account, reason = t(why));
        }
        if rest == "needs its password" {
            return tf!("{account} needs its password", account = account);
        }
        if let Some(why) = rest.strip_prefix("needs you to sign in again: ") {
            return tf!("{account} needs you to sign in again: {reason}", account = account, reason = t(why));
        }
        if rest == "needs you to sign in again" {
            return tf!("{account} needs you to sign in again", account = account);
        }
    }
    // A mailbox's notice: « {account}: message ».
    if let Some((account, message)) = text.split_once(": ").filter(|(a, _)| a.contains('@') && !a.contains(char::is_whitespace)) {
        return tf!("{account}: {message}", account = account, message = t(message));
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::translate;
    use crate::i18n::{Lang, set_current};

    #[test]
    fn the_engine_s_messages_show_in_the_interface_language() {
        set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
        let refused = translate("The server refused a change: sign-in failed: the server refused the user name or password");
        assert!(!refused.contains("The server refused a change") && !refused.contains("sign-in failed"), "{refused}");
        assert!(!refused.contains("the server refused the user name or password"), "the reason too: {refused}");
        let page = translate("jane@example.ch needs its password: the page gave no new access to this mailbox: reload it");
        assert!(page.contains("jane@example.ch") && !page.contains("needs its password") && !page.contains("reload it"), "{page}");
        let typed = translate("jane@example.ch needs its password: enter the password: the browser doesn't keep it");
        assert!(!typed.contains("the browser doesn't keep it"), "{typed}");
        let undone = translate("Ann Lee: its address book can't be changed any more, so the change made here was undone");
        assert!(undone.contains("Ann Lee") && !undone.contains("can't be changed"), "{undone}");
        let linked = translate("calendars and contacts: a contact was not deleted: Ce contact est lié à des documents");
        assert!(linked.ends_with("Ce contact est lié à des documents") && !linked.contains("was not deleted"), "{linked}");
        assert_eq!(translate("something else entirely"), "something else entirely", "an unknown message as written");
        // In English, as written.
        set_current(Lang::EN);
        let english = "The server refused a change: sign-in failed: the server refused the user name or password";
        assert_eq!(translate(english), english);
        assert_eq!(translate("jane@example.ch: the draft was saved"), "jane@example.ch: the draft was saved");
    }
}
