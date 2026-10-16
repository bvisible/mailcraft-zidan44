// //// Neoffice — added file (no upstream equivalent): the translations of what the Neoffice fork adds
// (Nora), kept out of the upstream catalogs so that merging upstream never touches them.

//! The Neoffice fork's interface strings in every language of [`super::LANGUAGES`]: the English,
//! then the translation for each language code. The catalog of a language adds them to its own
//! entries (`LangInfo::catalog`).

/// (English, [(language code, translation)]).
type Entry = (&'static str, &'static [(&'static str, &'static str)]);

const STRINGS: &[Entry] = &[
    (
        "Nora",
        &[
            ("es", "Nora"),
            ("fr", "Nora"),
            ("de", "Nora"),
            ("pt-br", "Nora"),
            ("it", "Nora"),
            ("ru", "Nora"),
            ("ja", "Nora"),
            ("ko", "Nora"),
            ("zh-hans", "Nora"),
            ("zh-hant", "Nora"),
            ("ar", "Nora"),
            ("he", "Nora"),
        ],
    ),
    (
        "Nora, the Neoffice assistant",
        &[
            ("es", "Nora, la asistente de Neoffice"),
            ("fr", "Nora, l’assistante Neoffice"),
            ("de", "Nora, die Neoffice-Assistentin"),
            ("pt-br", "Nora, a assistente do Neoffice"),
            ("it", "Nora, l'assistente di Neoffice"),
            ("ru", "Nora, помощница Neoffice"),
            ("ja", "Nora、Neoffice のアシスタント"),
            ("ko", "Nora, Neoffice 어시스턴트"),
            ("zh-hans", "Nora，Neoffice 助手"),
            ("zh-hant", "Nora，Neoffice 助理"),
            ("ar", "Nora، مساعدة Neoffice"),
            ("he", "Nora, העוזרת של Neoffice"),
        ],
    ),
    (
        "Reply with Nora",
        &[
            ("es", "Responder con Nora"),
            ("fr", "Répondre avec Nora"),
            ("de", "Mit Nora antworten"),
            ("pt-br", "Responder com Nora"),
            ("it", "Rispondi con Nora"),
            ("ru", "Ответить с помощью Nora"),
            ("ja", "Nora で返信"),
            ("ko", "Nora로 회신"),
            ("zh-hans", "以 Nora 回复"),
            ("zh-hant", "以 Nora 回覆"),
            ("ar", "الرد باستخدام Nora"),
            ("he", "השב עם Nora"),
        ],
    ),
    (
        "Reply all with Nora",
        &[
            ("es", "Responder a todos con Nora"),
            ("fr", "Répondre à tous avec Nora"),
            ("de", "Allen mit Nora antworten"),
            ("pt-br", "Responder a todos com Nora"),
            ("it", "Rispondi a tutti con Nora"),
            ("ru", "Ответить всем с помощью Nora"),
            ("ja", "Nora で全員に返信"),
            ("ko", "Nora로 전체 회신"),
            ("zh-hans", "以 Nora 全部回复"),
            ("zh-hant", "以 Nora 全部回覆"),
            ("ar", "الرد على الكل باستخدام Nora"),
            ("he", "השב לכולם עם Nora"),
        ],
    ),
    (
        "Nora is already writing a reply.",
        &[
            ("es", "Nora ya está escribiendo una respuesta."),
            ("fr", "Nora rédige déjà une réponse."),
            ("de", "Nora schreibt bereits eine Antwort."),
            ("pt-br", "A Nora já está escrevendo uma resposta."),
            ("it", "Nora sta già scrivendo una risposta."),
            ("ru", "Nora уже пишет ответ."),
            ("ja", "Nora がすでに返信を作成中です。"),
            ("ko", "Nora가 이미 회신을 작성하고 있습니다."),
            ("zh-hans", "Nora 已经在撰写回复。"),
            ("zh-hant", "Nora 已經在撰寫回覆。"),
            ("ar", "Nora تكتب ردًا بالفعل."),
            ("he", "Nora כבר כותבת תשובה."),
        ],
    ),
    (
        "Nora is writing the reply…",
        &[
            ("es", "Nora está escribiendo la respuesta…"),
            ("fr", "Nora rédige la réponse…"),
            ("de", "Nora schreibt die Antwort…"),
            ("pt-br", "A Nora está escrevendo a resposta…"),
            ("it", "Nora sta scrivendo la risposta…"),
            ("ru", "Nora пишет ответ…"),
            ("ja", "Nora が返信を作成しています…"),
            ("ko", "Nora가 회신을 작성하고 있습니다…"),
            ("zh-hans", "Nora 正在撰写回复…"),
            ("zh-hant", "Nora 正在撰寫回覆…"),
            ("ar", "Nora تكتب الرد…"),
            ("he", "Nora כותבת את התשובה…"),
        ],
    ),
    (
        "Nora couldn't be reached: {reason}",
        &[
            ("es", "No se pudo contactar con Nora: {reason}"),
            ("fr", "Impossible de joindre Nora : {reason}"),
            ("de", "Nora ist nicht erreichbar: {reason}"),
            ("pt-br", "Não foi possível contatar a Nora: {reason}"),
            ("it", "Impossibile contattare Nora: {reason}"),
            ("ru", "Не удалось связаться с Nora: {reason}"),
            ("ja", "Nora に接続できませんでした：{reason}"),
            ("ko", "Nora에 연결할 수 없습니다: {reason}"),
            ("zh-hans", "无法连接到 Nora：{reason}"),
            ("zh-hant", "無法連線到 Nora：{reason}"),
            ("ar", "تعذّر الوصول إلى Nora: {reason}"),
            ("he", "לא ניתן להגיע ל-Nora: {reason}"),
        ],
    ),
    (
        "Nora couldn't write the reply: {reason}",
        &[
            ("es", "Nora no pudo escribir la respuesta: {reason}"),
            ("fr", "Nora n’a pas pu rédiger la réponse : {reason}"),
            ("de", "Nora konnte die Antwort nicht verfassen: {reason}"),
            ("pt-br", "A Nora não conseguiu escrever a resposta: {reason}"),
            ("it", "Nora non è riuscita a scrivere la risposta: {reason}"),
            ("ru", "Nora не смогла написать ответ: {reason}"),
            ("ja", "Nora が返信を作成できませんでした：{reason}"),
            ("ko", "Nora가 회신을 작성하지 못했습니다: {reason}"),
            ("zh-hans", "Nora 无法撰写回复：{reason}"),
            ("zh-hant", "Nora 無法撰寫回覆：{reason}"),
            ("ar", "تعذّر على Nora كتابة الرد: {reason}"),
            ("he", "Nora לא הצליחה לכתוב את התשובה: {reason}"),
        ],
    ),
    (
        "The reply was closed before Nora's draft came back.",
        &[
            ("es", "La respuesta se cerró antes de que llegara el borrador de Nora."),
            ("fr", "La réponse a été fermée avant l’arrivée du brouillon de Nora."),
            ("de", "Die Antwort wurde geschlossen, bevor der Entwurf von Nora eintraf."),
            ("pt-br", "A resposta foi fechada antes que o rascunho da Nora chegasse."),
            ("it", "La risposta è stata chiusa prima che la bozza di Nora arrivasse."),
            ("ru", "Ответ был закрыт до того, как пришёл черновик от Nora."),
            ("ja", "Nora の下書きが届く前に返信画面が閉じられました。"),
            ("ko", "Nora의 초안이 도착하기 전에 회신이 닫혔습니다."),
            ("zh-hans", "在 Nora 的草稿返回之前，回复已被关闭。"),
            ("zh-hant", "在 Nora 的草稿返回之前，回覆已被關閉。"),
            ("ar", "أُغلق الرد قبل وصول مسودة Nora."),
            ("he", "התשובה נסגרה לפני שהטיוטה של Nora חזרה."),
        ],
    ),
    (
        "Nora's draft is in the reply: read it before sending.",
        &[
            ("es", "El borrador de Nora está en la respuesta: léalo antes de enviar."),
            ("fr", "Le brouillon de Nora est dans la réponse : relisez-le avant de l’envoyer."),
            ("de", "Der Entwurf von Nora ist in der Antwort: Lesen Sie ihn vor dem Senden durch."),
            ("pt-br", "O rascunho da Nora está na resposta: leia-o antes de enviar."),
            ("it", "La bozza di Nora è nella risposta: leggila prima di inviare."),
            ("ru", "Черновик Nora находится в ответе: прочитайте его перед отправкой."),
            ("ja", "Nora の下書きが返信に入っています。送信する前に内容を確認してください。"),
            ("ko", "Nora의 초안이 회신에 들어 있습니다. 보내기 전에 내용을 확인하세요."),
            ("zh-hans", "Nora 的草稿已在回复中，发送前请先阅读。"),
            ("zh-hant", "Nora 的草稿已在回覆中，傳送前請先閱讀。"),
            ("ar", "مسودة Nora موجودة في الرد: اقرأها قبل الإرسال."),
            ("he", "הטיוטה של Nora נמצאת בתשובה: קרא אותה לפני השליחה."),
        ],
    ),
];

/// This fork's entries for the language `code`: (English, translation).
pub(super) fn entries(code: &str) -> impl Iterator<Item = (String, String)> + '_ {
    STRINGS
        .iter()
        .filter_map(move |(english, all)| all.iter().find(|(lang, _)| *lang == code).map(|(_, text)| ((*english).to_string(), (*text).to_string())))
}

#[cfg(test)]
mod tests {
    use super::super::{LANGUAGES, placeholders};
    use super::STRINGS;

    #[test]
    fn every_string_is_in_every_language_with_its_placeholders() {
        assert!(!STRINGS.is_empty());
        for (english, all) in STRINGS {
            for lang in LANGUAGES.iter().skip(1) {
                let text = all.iter().find(|(code, _)| *code == lang.code).map(|(_, text)| *text);
                let Some(text) = text else { panic!("{english:?} has no {} translation", lang.code) };
                assert!(!text.trim().is_empty() && text.trim() == text, "{english:?} in {}", lang.code);
                assert_eq!(placeholders(english), placeholders(text), "{english:?} in {}", lang.code);
                assert_eq!(english.ends_with('…'), text.ends_with('…'), "{english:?} in {}", lang.code);
            }
            assert_eq!(all.len(), LANGUAGES.len() - 1, "{english:?}: one translation per language, no more");
        }
    }
}
