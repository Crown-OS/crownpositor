//! The wording of each consent question, and what its answer means.

use crownos_config::consent_dialog::{ConsentOption, ConsentQuestion};

use super::consent_dialog::ConsentOutcome;

const REMEMBER: &str = "Remember this choice";

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
        grant: "Allow".into(),
        deny: "Don't Allow".into(),
        options: vec![ConsentOption::Checkbox {
            label: REMEMBER.into(),
            initially_on: false,
        }],
    }
}

/// `Err` when nobody could be asked.
pub fn access_answer(outcome: &ConsentOutcome) -> Result<AccessAnswer, String> {
    match outcome {
        ConsentOutcome::Answered(answer) => Ok(AccessAnswer {
            granted: answer.granted,
            remember: answer.checked(0),
        }),
        ConsentOutcome::TimedOut => Ok(AccessAnswer {
            granted: false,
            remember: false,
        }),
        ConsentOutcome::Withdrawn | ConsentOutcome::Unavailable => {
            Err("no consent dialog could ask".to_owned())
        }
    }
}

pub fn context_access_question(app_name: &str, app_key: &str) -> ConsentQuestion {
    ConsentQuestion {
        dangerous: false,
        title: format!("Let {app_name} see what you do?"),
        message: format!(
            "{app_key} asks to know which windows are open and which one you use. \
             It never sees the contents of a window. You can take this back any time in Settings."
        ),
        grant: "Allow".into(),
        deny: "Don't Allow".into(),
        options: vec![
            ConsentOption::Checkbox {
                label: REMEMBER.into(),
                initially_on: false,
            },
            ConsentOption::List {
                label: "The app may see".into(),
                items: vec!["Window list".into(), "Window list and titles".into()],
                initial: 0,
            },
        ],
    }
}

/// `Err` when nobody could be asked.
pub fn context_answer(outcome: &ConsentOutcome) -> Result<ContextAnswer, String> {
    match outcome {
        ConsentOutcome::Answered(answer) => Ok(ContextAnswer {
            granted: answer.granted,
            remember: answer.checked(0),
            titles: answer.chosen(1) == Some(1),
        }),
        ConsentOutcome::TimedOut => Ok(ContextAnswer {
            granted: false,
            remember: false,
            titles: false,
        }),
        ConsentOutcome::Withdrawn | ConsentOutcome::Unavailable => {
            Err("no consent dialog could ask".to_owned())
        }
    }
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
        grant: "Allow".into(),
        deny: "Deny".into(),
        options: Vec::new(),
    }
}

/// crownos-secretsd asking before an agent uses secrets for a run or a
/// proxy. Each text is display text only, already cleaned.
#[derive(Debug, Clone)]
pub struct SecretUsePrompt {
    pub agent_app_key: String,
    pub secret_names: String,
    pub proxy: bool,
    pub detail: String,
}

pub fn secret_use_question(prompt: &SecretUsePrompt) -> ConsentQuestion {
    let purpose = if prompt.proxy {
        "add them to HTTPS requests to"
    } else {
        "run the command template"
    };
    ConsentQuestion {
        dangerous: true,
        title: format!("Allow an agent to use {}?", prompt.secret_names),
        message: format!(
            "{} asks via crownos-secretsd to {purpose} {}.\n\n\
             The agent does not see the values.",
            prompt.agent_app_key, prompt.detail
        ),
        grant: "Allow".into(),
        deny: "Deny".into(),
        options: Vec::new(),
    }
}

/// Whether the user allowed the typing or the use.
pub fn typing_allowed(outcome: &ConsentOutcome) -> bool {
    outcome.granted()
}

#[cfg(test)]
mod tests {
    use crownos_config::consent_dialog::DialogAnswer;

    use super::*;

    fn answered(question: &ConsentQuestion, choice: usize, values: Vec<usize>) -> ConsentOutcome {
        let status = question.shape().encode(&DialogAnswer {
            choice,
            option_values: values,
        });
        question
            .answer(status.map(i32::from))
            .map_or(ConsentOutcome::Unavailable, ConsentOutcome::Answered)
    }

    #[test]
    fn every_context_answer_keeps_remember_and_titles_apart() {
        let question = context_access_question("Contextd", "exe:/usr/bin/crown-contextd");
        let cases = [
            ((0, vec![0, 0]), (true, false, false)),
            ((0, vec![0, 1]), (true, false, true)),
            ((0, vec![1, 0]), (true, true, false)),
            ((0, vec![1, 1]), (true, true, true)),
            ((1, vec![1, 0]), (false, true, false)),
        ];
        for ((choice, values), (granted, remember, titles)) in cases {
            assert_eq!(
                context_answer(&answered(&question, choice, values)),
                Ok(ContextAnswer {
                    granted,
                    remember,
                    titles
                })
            );
        }
        assert!(context_answer(&ConsentOutcome::Unavailable).is_err());
    }

    #[test]
    fn agent_access_keeps_remember_on_a_deny() {
        let question = agent_access_question("org.example.App", "");
        assert_eq!(
            access_answer(&answered(&question, 1, vec![1])),
            Ok(AccessAnswer {
                granted: false,
                remember: true
            })
        );
        assert_eq!(
            access_answer(&answered(&question, 0, vec![0])),
            Ok(AccessAnswer {
                granted: true,
                remember: false
            })
        );
        assert!(access_answer(&ConsentOutcome::Withdrawn).is_err());
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
        assert!(typing_allowed(&answered(&question, 0, Vec::new())));
        assert!(!typing_allowed(&answered(&question, 1, Vec::new())));
    }
}
