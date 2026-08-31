use futures::channel::oneshot;
use gpui::{App, AsyncApp};
use mezon_store::Settings;

use crate::app::shell::Shell;

pub(crate) const TOAST_KEY: &str = "file-dialog-unavailable";

const CLOSED_WITHOUT_ANSWERING: &str = "the file dialog closed without answering";

enum Outcome<T> {
    Picked(T),
    Cancelled,
    Unavailable(Option<String>),
}

fn classify<T>(result: Result<anyhow::Result<Option<T>>, oneshot::Canceled>) -> Outcome<T> {
    match result {
        Ok(Ok(Some(picked))) => Outcome::Picked(picked),
        Ok(Ok(None)) => Outcome::Cancelled,
        Ok(Err(error)) => Outcome::Unavailable(Some(error.to_string())),
        Err(_) => Outcome::Unavailable(None),
    }
}

pub(crate) async fn resolve<T>(
    receiver: oneshot::Receiver<anyhow::Result<Option<T>>>,
    cx: &AsyncApp,
) -> Option<T> {
    match classify(receiver.await) {
        Outcome::Picked(picked) => Some(picked),
        Outcome::Cancelled => None,
        Outcome::Unavailable(reason) => {
            tracing::warn!(
                "file dialog unavailable: {}",
                reason.as_deref().unwrap_or(CLOSED_WITHOUT_ANSWERING)
            );
            cx.update(|cx| {
                let message = unavailable_message(cx);
                if let Some(shell) = Shell::try_global(cx) {
                    shell.update(cx, |shell, cx| shell.error_once(TOAST_KEY, message, cx));
                }
            });
            None
        }
    }
}

pub(crate) fn unavailable_message(cx: &App) -> &'static str {
    let locale = Settings::try_global(cx)
        .map(|settings| settings.read(cx).language.clone())
        .unwrap_or_default();
    message_for(&locale)
}

fn message_for(locale: &str) -> &'static str {
    let key = if cfg!(target_os = "linux") {
        "file.dialogPortalMissing"
    } else {
        "file.dialogUnavailable"
    };
    mezon_i18n::t(locale, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const PORTAL_MISSING: &str =
        "Couldn't open file picker due to missing xdg-desktop-portal implementation.";

    fn picked(
        paths: Vec<PathBuf>,
    ) -> Result<anyhow::Result<Option<Vec<PathBuf>>>, oneshot::Canceled> {
        Ok(Ok(Some(paths)))
    }

    #[test]
    fn a_chosen_path_comes_back_untouched() {
        let chosen = vec![PathBuf::from("/tmp/ca.crt")];
        match classify(picked(chosen.clone())) {
            Outcome::Picked(paths) => assert_eq!(paths, chosen),
            _ => panic!("choosing a file must not be treated as a failure"),
        }
    }

    #[test]
    fn cancelling_the_dialog_is_an_answer_not_a_failure() {
        let cancelled: Result<anyhow::Result<Option<Vec<PathBuf>>>, oneshot::Canceled> =
            Ok(Ok(None));
        match classify(cancelled) {
            Outcome::Cancelled => {}
            Outcome::Picked(_) => panic!("a cancelled dialog has no path to hand back"),
            Outcome::Unavailable(reason) => {
                panic!("dismissing the dialog must stay silent, got a toast saying {reason:?}")
            }
        }
    }

    #[test]
    fn a_dialog_that_never_opened_keeps_the_reason_it_gave() {
        let failed: Result<anyhow::Result<Option<Vec<PathBuf>>>, oneshot::Canceled> =
            Ok(Err(anyhow::anyhow!(PORTAL_MISSING)));
        match classify(failed) {
            Outcome::Unavailable(Some(reason)) => assert_eq!(reason, PORTAL_MISSING),
            _ => panic!("a picker that could not run must be reported with its reason"),
        }
    }

    #[test]
    fn a_dialog_dropped_without_answering_is_reported_with_no_reason_to_show() {
        let (sender, mut receiver) = oneshot::channel::<anyhow::Result<Option<Vec<PathBuf>>>>();
        drop(sender);
        let dropped = receiver
            .try_recv()
            .map(|value| value.expect("a dropped sender delivers nothing"));
        match classify(dropped) {
            Outcome::Unavailable(None) => {}
            Outcome::Unavailable(Some(reason)) => {
                panic!("the platform gave no reason here, so none may be invented: got {reason:?}")
            }
            _ => panic!("losing the dialog channel leaves the user with no feedback otherwise"),
        }
    }

    #[test]
    fn the_toast_is_one_translated_sentence_with_no_raw_platform_text() {
        for locale in ["vi", "en", "jpn", "does-not-exist"] {
            let message = message_for(locale);
            assert!(
                !message.contains("xdg-desktop-portal implementation"),
                "{locale} leaks gpui's raw sentence into a toast that already says it: {message}"
            );
            assert!(
                !message.contains(": Couldn't") && !message.contains(": Could not"),
                "{locale} states the same failure twice: {message}"
            );
            assert!(
                !message.ends_with(':'),
                "{locale} ends mid-sentence: {message}"
            );
        }
    }

    #[test]
    fn every_locale_translates_the_message_this_platform_shows() {
        for locale in [
            "vi", "en", "ru", "ukr", "es", "tt", "de", "it", "pt", "jpn", "pl", "kr", "swe", "blr",
            "fr", "nl",
        ] {
            let message = message_for(locale);
            assert!(
                !message.starts_with("file.dialog"),
                "locale {locale} falls through to the raw key: {message}"
            );
        }
    }

    #[test]
    fn linux_tells_the_user_what_to_install() {
        let message = message_for("en");
        if cfg!(target_os = "linux") {
            assert!(
                message.contains("xdg-desktop-portal"),
                "the only fix is installing a portal backend, so name it: {message}"
            );
        } else {
            assert!(
                !message.contains("xdg-desktop-portal"),
                "a portal is a Linux concept and means nothing here: {message}"
            );
        }
    }
}
