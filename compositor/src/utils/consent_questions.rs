//! The wording of each consent question, and what each button means.

use super::consent_dialog::{ConsentAnswer, ConsentQuestion};

const ALLOW: &str = "allow";
const ALLOW_ALWAYS: &str = "allow-always";
const DENY_ALWAYS: &str = "deny-always";
const WINDOW_LIST: &str = "metadata";
const WINDOW_LIST_ALWAYS: &str = "metadata-always";
const TITLES: &str = "text";
const TITLES_ALWAYS: &str = "text-always";
const TYPE_SECRET: &str = "grant";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessAnswer {
    pub granted: bool,
    pub remember: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAnswer {
    pub granted: bool,
    pub remember: bool,
    /// The user also allowed window titles.
    pub titles: bool,
}

/// crownos-secretsd asking to type a secret for an agent. Each text is
/// display text only, already cleaned of control characters.
#[derive(Debug, Clone)]
pub struct SecretTypingPrompt {
    pub agent: String,
    pub secret_names: String,
    /// The name and the executable of the window's verified app.
    pub app_name: String,
    pub app_key: String,
    pub window_title: String,
    pub origin: String,
}

pub fn agent_access_question(app_id: &str, reason: &str) -> ConsentQuestion {
    let reason = if reason.is_empty() {
        "The app asks to show its interface to AI agents."
    } else {
        reason
    };
    ConsentQuestion {
        dangerous: true,
        title: format!("Let AI agents use {app_id}?"),
        message: format!(
            "{reason}\n\nAgents will see everything in the app and can click and type in it. \
             You can take this back any time in Settings."
        ),
        decline: "Don't Allow",
        choices: vec![
            (ALLOW, "Allow"),
            (ALLOW_ALWAYS, "Always Allow"),
            (DENY_ALWAYS, "Never Allow"),
        ],
    }
}

/// `Err` when nobody could be asked.
pub fn access_answer(answer: &ConsentAnswer) -> Result<AccessAnswer, String> {
    let (granted, remember) = match answer {
        ConsentAnswer::Chosen(ALLOW) => (true, false),
        ConsentAnswer::Chosen(ALLOW_ALWAYS) => (true, true),
        ConsentAnswer::Chosen(DENY_ALWAYS) => (false, true),
        ConsentAnswer::Chosen(_) | ConsentAnswer::Declined | ConsentAnswer::TimedOut => {
            (false, false)
        }
        ConsentAnswer::Withdrawn | ConsentAnswer::Unavailable => {
            return Err("no consent dialog could ask".to_owned());
        }
    };
    Ok(AccessAnswer { granted, remember })
}

pub fn context_access_question(app_name: &str, app_key: &str) -> ConsentQuestion {
    ConsentQuestion {
        dangerous: false,
        title: format!("Let {app_name} see what you do?"),
        message: format!(
            "{app_key} asks to know which windows are open and which one you use. \
             It never sees the contents of a window. You can take this back any time in Settings."
        ),
        decline: "Don't Allow",
        choices: vec![
            (WINDOW_LIST, "Allow Window List"),
            (TITLES, "Allow Window List and Titles"),
            (WINDOW_LIST_ALWAYS, "Always Allow Window List"),
            (TITLES_ALWAYS, "Always Allow Window List and Titles"),
            (DENY_ALWAYS, "Never Allow"),
        ],
    }
}

/// `Err` when nobody could be asked.
pub fn context_answer(answer: &ConsentAnswer) -> Result<ContextAnswer, String> {
    let (granted, remember, titles) = match answer {
        ConsentAnswer::Chosen(WINDOW_LIST) => (true, false, false),
        ConsentAnswer::Chosen(TITLES) => (true, false, true),
        ConsentAnswer::Chosen(WINDOW_LIST_ALWAYS) => (true, true, false),
        ConsentAnswer::Chosen(TITLES_ALWAYS) => (true, true, true),
        ConsentAnswer::Chosen(DENY_ALWAYS) => (false, true, false),
        ConsentAnswer::Chosen(_) | ConsentAnswer::Declined | ConsentAnswer::TimedOut => {
            (false, false, false)
        }
        ConsentAnswer::Withdrawn | ConsentAnswer::Unavailable => {
            return Err("no consent dialog could ask".to_owned());
        }
    };
    Ok(ContextAnswer {
        granted,
        remember,
        titles,
    })
}

pub fn secret_typing_question(prompt: &SecretTypingPrompt) -> ConsentQuestion {
    let target = if prompt.window_title.is_empty() {
        prompt.app_name.as_str()
    } else {
        prompt.window_title.as_str()
    };
    let origin = if prompt.origin.is_empty() {
        "none"
    } else {
        prompt.origin.as_str()
    };
    ConsentQuestion {
        dangerous: true,
        title: format!("Type a secret into “{target}”?"),
        message: format!(
            "{} asks via crownos-secretsd to type {}.\n\n\
             App: {} ({})\nWindow: {}\nPage, as the agent reports it: {origin}\n\n\
             The agent does not see the value. Allow only if you asked the agent to sign in here.",
            prompt.agent, prompt.secret_names, prompt.app_name, prompt.app_key, prompt.window_title
        ),
        decline: "Deny",
        choices: vec![(TYPE_SECRET, "Allow")],
    }
}

/// Whether the user allowed the typing.
pub fn typing_allowed(answer: &ConsentAnswer) -> bool {
    *answer == ConsentAnswer::Chosen(TYPE_SECRET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_context_choice_keeps_remember_and_titles_apart() {
        let question = context_access_question("Contextd", "exe:/usr/bin/crown-contextd");
        let answers: Vec<_> = question
            .choices
            .iter()
            .map(|(id, _)| context_answer(&ConsentAnswer::Chosen(id)))
            .collect();
        assert_eq!(
            answers,
            [
                Ok(ContextAnswer {
                    granted: true,
                    remember: false,
                    titles: false
                }),
                Ok(ContextAnswer {
                    granted: true,
                    remember: false,
                    titles: true
                }),
                Ok(ContextAnswer {
                    granted: true,
                    remember: true,
                    titles: false
                }),
                Ok(ContextAnswer {
                    granted: true,
                    remember: true,
                    titles: true
                }),
                Ok(ContextAnswer {
                    granted: false,
                    remember: true,
                    titles: false
                }),
            ]
        );
        assert!(context_answer(&ConsentAnswer::Unavailable).is_err());
        assert_eq!(
            context_answer(&ConsentAnswer::TimedOut),
            Ok(ContextAnswer {
                granted: false,
                remember: false,
                titles: false
            })
        );
    }

    #[test]
    fn agent_access_choices_map_to_grant_and_remember() {
        assert_eq!(
            access_answer(&ConsentAnswer::Chosen(ALLOW_ALWAYS)),
            Ok(AccessAnswer {
                granted: true,
                remember: true
            })
        );
        assert_eq!(
            access_answer(&ConsentAnswer::Declined),
            Ok(AccessAnswer {
                granted: false,
                remember: false
            })
        );
        assert!(access_answer(&ConsentAnswer::Withdrawn).is_err());
    }

    #[test]
    fn the_typing_question_names_the_verified_app() {
        let question = secret_typing_question(&SecretTypingPrompt {
            agent: "claude".into(),
            secret_names: "github".into(),
            app_name: "chromium".into(),
            app_key: "exe:/usr/lib/chromium/chromium".into(),
            window_title: "Sign in".into(),
            origin: String::new(),
        });
        assert!(question.message.contains("exe:/usr/lib/chromium/chromium"));
        assert!(question.message.contains("via crownos-secretsd"));
        assert!(typing_allowed(&ConsentAnswer::Chosen(TYPE_SECRET)));
        assert!(!typing_allowed(&ConsentAnswer::Declined));
    }
}
