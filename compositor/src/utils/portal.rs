//! Asking the user through the desktop portal, which draws its dialog where
//! no application can click it.

use std::collections::HashMap;

use zbus::{
    Connection,
    zvariant::{ObjectPath, OwnedValue, Value},
};

/// Portal backends that implement the access dialog, most specific first.
const BACKENDS: [&str; 2] = [
    "org.freedesktop.impl.portal.desktop.crownos",
    "org.freedesktop.impl.portal.desktop.gtk",
];
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.impl.portal.Access";
const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
const REMEMBER: &str = "remember";
const CONTEXT_SCOPE: &str = "scope";
const METADATA: &str = "metadata";
const TEXT: &str = "text";
const GRANTED: u32 = 0;

/// One `a(ssa(ss)s)` entry of the `choices` option: id, label, items, initial.
type Choice<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>, &'a str);

/// An application asking to show its UI to agents.
#[derive(Debug, Clone)]
pub struct AccessPrompt {
    pub ticket: u64,
    pub app_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessAnswer {
    pub granted: bool,
    pub remember: bool,
}

/// An application asking for screen context for the first time.
#[derive(Debug, Clone)]
pub struct ContextPrompt {
    pub ticket: u64,
    /// The name the user knows the app by.
    pub app_name: String,
    /// The `AppKey` text, for the portal's records.
    pub app_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAnswer {
    pub granted: bool,
    pub remember: bool,
    /// The user also allowed window titles.
    pub titles: bool,
}

/// One question for the Access portal.
struct AccessDialog<'a> {
    /// The object path of the portal request, unique among open dialogs.
    request_path: String,
    app_id: &'a str,
    title: &'a str,
    subtitle: &'a str,
    body: &'a str,
    choices: Vec<Choice<'a>>,
}

/// Whether the user granted, with the answers to the choices.
struct DialogAnswer {
    granted: bool,
    choices: Vec<(String, String)>,
}

impl DialogAnswer {
    fn choice(&self, id: &str) -> Option<&str> {
        self.choices
            .iter()
            .find(|(choice, _)| choice == id)
            .map(|(_, value)| value.as_str())
    }
}

pub async fn ask_agent_access(prompt: AccessPrompt) -> Result<AccessAnswer, String> {
    let title = format!("Let AI agents use {}?", prompt.app_id);
    let subtitle = if prompt.reason.is_empty() {
        "The app asks to show its interface to AI agents.".to_owned()
    } else {
        prompt.reason.clone()
    };
    let answer = ask(AccessDialog {
        request_path: format!("/org/crownos/portal/request/agent/{}", prompt.ticket),
        app_id: &prompt.app_id,
        title: &title,
        subtitle: &subtitle,
        body: "Agents will see everything in the app and can click and type in it. \
               You can take this back any time in Settings.",
        choices: vec![(REMEMBER, "Remember this choice", Vec::new(), "false")],
    })
    .await?;
    Ok(AccessAnswer {
        granted: answer.granted,
        remember: answer.choice(REMEMBER) == Some("true"),
    })
}

pub async fn ask_context_access(prompt: ContextPrompt) -> Result<ContextAnswer, String> {
    let title = format!("Let {} see what you do?", prompt.app_name);
    let subtitle = format!(
        "{} asks to know which windows are open and which one you use.",
        prompt.app_key
    );
    let answer = ask(AccessDialog {
        request_path: format!("/org/crownos/portal/request/context/{}", prompt.ticket),
        app_id: &prompt.app_key,
        title: &title,
        subtitle: &subtitle,
        body: "It never sees the contents of a window. \
               You can take this back any time in Settings.",
        choices: vec![
            (REMEMBER, "Remember this choice", Vec::new(), "false"),
            (
                CONTEXT_SCOPE,
                "The app may see",
                vec![(METADATA, "Window list"), (TEXT, "Window list and titles")],
                METADATA,
            ),
        ],
    })
    .await?;
    Ok(ContextAnswer {
        granted: answer.granted,
        remember: answer.choice(REMEMBER) == Some("true"),
        titles: answer.choice(CONTEXT_SCOPE) == Some(TEXT),
    })
}

async fn ask(dialog: AccessDialog<'_>) -> Result<DialogAnswer, String> {
    let connection = Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    let handle =
        ObjectPath::try_from(dialog.request_path.as_str()).map_err(|error| error.to_string())?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("modal", true.into());
    options.insert("grant_label", "Allow".into());
    options.insert("deny_label", "Don't allow".into());
    options.insert("icon", "dialog-password-symbolic".into());
    options.insert("choices", Value::new(dialog.choices));
    for backend in BACKENDS {
        let reply = connection
            .call_method(
                Some(backend),
                PATH,
                Some(INTERFACE),
                "AccessDialog",
                &(
                    &handle,
                    dialog.app_id,
                    "",
                    dialog.title,
                    dialog.subtitle,
                    dialog.body,
                    &options,
                ),
            )
            .await;
        match reply {
            Ok(reply) => {
                let (response, results): (u32, HashMap<String, OwnedValue>) = reply
                    .body()
                    .deserialize()
                    .map_err(|error| error.to_string())?;
                let choices = results
                    .get("choices")
                    .and_then(|choices| <Vec<(String, String)>>::try_from(choices.clone()).ok())
                    .unwrap_or_default();
                return Ok(DialogAnswer {
                    granted: response == GRANTED,
                    choices,
                });
            }
            Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == SERVICE_UNKNOWN => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("no portal backend offers an access dialog".to_owned())
}
