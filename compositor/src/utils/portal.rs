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
const GRANTED: u32 = 0;

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

pub async fn ask_agent_access(prompt: AccessPrompt) -> Result<AccessAnswer, String> {
    let connection = Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    let handle = ObjectPath::try_from(format!("/org/crownos/portal/request/{}", prompt.ticket))
        .map_err(|error| error.to_string())?;
    let title = format!("Let AI agents use {}?", prompt.app_id);
    let subtitle = if prompt.reason.is_empty() {
        "The app asks to show its interface to AI agents.".to_owned()
    } else {
        prompt.reason.clone()
    };
    let body = "Agents will see everything in the app and can click and type in it. \
                You can take this back any time in Settings.";
    let choices: Vec<(&str, &str, Vec<(&str, &str)>, &str)> =
        vec![(REMEMBER, "Remember this choice", Vec::new(), "false")];
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("modal", true.into());
    options.insert("grant_label", "Allow".into());
    options.insert("deny_label", "Don't allow".into());
    options.insert("icon", "dialog-password-symbolic".into());
    options.insert("choices", Value::new(choices));
    for backend in BACKENDS {
        let reply = connection
            .call_method(
                Some(backend),
                PATH,
                Some(INTERFACE),
                "AccessDialog",
                &(
                    &handle,
                    prompt.app_id.as_str(),
                    "",
                    title.as_str(),
                    subtitle.as_str(),
                    body,
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
                let remember = results
                    .get("choices")
                    .and_then(|choices| <Vec<(String, String)>>::try_from(choices.clone()).ok())
                    .is_some_and(|choices| {
                        choices
                            .iter()
                            .any(|(id, value)| id == REMEMBER && value == "true")
                    });
                return Ok(AccessAnswer {
                    granted: response == GRANTED,
                    remember,
                });
            }
            Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == SERVICE_UNKNOWN => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("no portal backend offers an access dialog".to_owned())
}
