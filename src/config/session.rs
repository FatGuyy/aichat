use super::input::resolve_data_url;
use super::role::Role;
use super::{Input, Model};

use crate::client::{Message, MessageContent, MessageRole};
use crate::render::MarkdownRender;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs::{self, read_to_string};
use std::path::Path;

// a constant representing the name used for temporary sessions.
pub const TEMP_SESSION_NAME: &str = "temp";

// this struct represents a session within the system,
// with its metadata, messages, and associated model and role
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Session {
    #[serde(rename(serialize = "model", deserialize = "model"))]
    model_id: String, // ID of the model associated with the session
    temperature: Option<f64>, // temperature to use for the session
    messages: Vec<Message>,   // vector storing the messages exchanged within the session
    #[serde(default)]
    data_urls: HashMap<String, String>, // hashmap storing data URLs
    #[serde(skip)]
    pub name: String, // name of the session
    #[serde(skip)]
    pub path: Option<String>, // path where the session is stored
    #[serde(skip)]
    pub dirty: bool, // boolean indicating whether the session has unsaved changes
    #[serde(skip)]
    pub role: Option<Role>, // optional Role associated with the session
    #[serde(skip)]
    pub model: Model, // model associated with the session.
}

impl Session {
    // this function creates a new session with the given name, model, and optional role
    pub fn new(name: &str, model: Model, role: Option<Role>) -> Self {
        // Setting temperature based on the role, if provided
        let temperature = role.as_ref().and_then(|v| v.temperature);
        Self {
            model_id: model.id(),
            temperature,
            messages: vec![],
            data_urls: Default::default(),
            name: name.to_string(),
            path: None,
            dirty: false,
            role,
            model,
        }
    }

    // this function loads a session from a yaml file located at the given path
    pub fn load(name: &str, path: &Path) -> Result<Self> {
        // this parses the yaml content into a Session struct
        let content = read_to_string(path)
            .with_context(|| format!("Failed to load session {} at {}", name, path.display()))?;
        let mut session: Self =
            serde_yaml::from_str(&content).with_context(|| format!("Invalid session {}", name))?;

        // this sets the session's name and path
        session.name = name.to_string();
        session.path = Some(path.display().to_string());

        // returnig the session inside a Result
        Ok(session)
    }

    // this function returns the name of the session
    pub fn name(&self) -> &str {
        &self.name
    }

    // this function returns the ID of the model associated with the session
    pub fn model(&self) -> &str {
        &self.model_id
    }

    // this funciton returns the sampling temperature used for the session
    pub fn temperature(&self) -> Option<f64> {
        self.temperature
    }

    // this function calculates and returns the total number of tokens in the session
    pub fn tokens(&self) -> usize {
        self.model.total_tokens(&self.messages)
    }

    // this function counts and returns the number of user messages in the session
    pub fn user_messages_len(&self) -> usize {
        self.messages.iter().filter(|v| v.role.is_user()).count()
    }

    // this function exports session information to a yaml string
    // It includes details such as path, model, temperature, total tokens, and messages
    pub fn export(&self) -> Result<String> {
        // checking is the guard is on
        self.guard_save()?;
        let (tokens, percent) = self.tokens_and_percent();
        // converting the data into json
        let mut data = json!({
            "path": self.path,
            "model": self.model(),
        });
        // is temp is given, we include it
        if let Some(temperature) = self.temperature() {
            data["temperature"] = temperature.into();
        }
        data["total_tokens"] = tokens.into();
        if let Some(max_tokens) = self.model.max_tokens {
            data["max_tokens"] = max_tokens.into();
        }
        if percent != 0.0 {
            data["total/max"] = format!("{}%", percent).into();
        }
        data["messages"] = json!(self.messages);

        // creating the yaml string for output
        let output = serde_yaml::to_string(&data)
            .with_context(|| format!("Unable to show info about session {}", &self.name))?;
        // returing the output wrapped in a result
        Ok(output)
    }

    // this function renders session information in markdown format
    // it includes details such as path, model, temperature, max tokens, and messages
    pub fn render(&self, render: &mut MarkdownRender) -> Result<String> {
        let mut items = vec![];

        if let Some(path) = &self.path {
            items.push(("path", path.to_string()));
        }

        items.push(("model", self.model.id()));

        if let Some(temperature) = self.temperature() {
            items.push(("temperature", temperature.to_string()));
        }

        if let Some(max_tokens) = self.model.max_tokens {
            items.push(("max_tokens", max_tokens.to_string()));
        }

        let mut lines: Vec<String> = items
            .iter()
            .map(|(name, value)| format!("{name:<20}{value}"))
            .collect();

        if !self.is_empty() {
            lines.push("".into());
            let resolve_url_fn = |url: &str| resolve_data_url(&self.data_urls, url.to_string());

            for message in &self.messages {
                match message.role {
                    MessageRole::System => {
                        continue;
                    }
                    MessageRole::Assistant => {
                        if let MessageContent::Text(text) = &message.content {
                            lines.push(render.render(text));
                        }
                        lines.push("".into());
                    }
                    MessageRole::User => {
                        lines.push(format!(
                            "{}）{}",
                            self.name,
                            message.content.render_input(resolve_url_fn)
                        ));
                    }
                }
            }
        }

        let output = lines.join("\n");
        Ok(output)
    }

    // this function calculates and returns the total number of tokens in the session along with the
    // percentage of tokens used relative to the maximum tokens allowed
    pub fn tokens_and_percent(&self) -> (usize, f32) {
        let tokens = self.tokens();
        let max_tokens = self.model.max_tokens.unwrap_or_default();
        let percent = if max_tokens == 0 {
            0.0
        } else {
            let percent = tokens as f32 / max_tokens as f32 * 100.0;
            (percent * 100.0).round() / 100.0
        };
        (tokens, percent)
    }

    // this function updates the role associated with the session
    pub fn update_role(&mut self, role: Option<Role>) -> Result<()> {
        self.guard_empty()?;
        // updating the session's temperature based on the new role, if provided
        self.temperature = role.as_ref().and_then(|v| v.temperature);
        self.role = role;
        Ok(())
    }

    // this function sets the temperature for the session
    pub fn set_temperature(&mut self, value: Option<f64>) {
        self.temperature = value;
    }

    // this funciton sets the model of the session
    pub fn set_model(&mut self, model: Model) -> Result<()> {
        self.model_id = model.id();
        self.model = model;
        Ok(())
    }

    // this funciton saves the session to a yaml file at the specified path
    pub fn save(&mut self, session_path: &Path) -> Result<()> {
        if !self.should_save() {
            return Ok(());
        }
        self.path = Some(session_path.display().to_string());

        // serializing the session into YAML format and writes it to the file
        let content = serde_yaml::to_string(&self)
            .with_context(|| format!("Failed to serde session {}", self.name))?;
        fs::write(session_path, content).with_context(|| {
            format!(
                "Failed to write session {} to {}",
                self.name,
                session_path.display()
            )
        })?;

        self.dirty = false;

        Ok(())
    }

    // this function checks if the session is saved
    pub fn should_save(&self) -> bool {
        !self.is_empty() && self.dirty
    }

    // this function guards against saving the session if the session path is not set
    pub fn guard_save(&self) -> Result<()> {
        if self.path.is_none() {
            // throwing an error if the session path is not set
            bail!("Not found session '{}'", self.name)
        }
        Ok(())
    }

    // this function guards against performing actions that require an empty session
    pub fn guard_empty(&self) -> Result<()> {
        if !self.is_empty() {
            // throwing an error if the session contains messages
            bail!("Cannot perform this action in a session with messages")
        }
        Ok(())
    }

    // this function checks if the session is temporary based on its name
    pub fn is_temp(&self) -> bool {
        self.name == TEMP_SESSION_NAME
    }

    // this function checks if the session is empty
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    // this function adds a message exchange(user input and assistant output) to the session
    pub fn add_message(&mut self, input: &Input, output: &str) -> Result<()> {
        // constructing and adding user and assistant messages based on the input and output provided
        let mut need_add_msg = true;
        if self.messages.is_empty() {
            if let Some(role) = self.role.as_ref() {
                self.messages.extend(role.build_messages(input));
                need_add_msg = false;
            }
        }
        if need_add_msg {
            self.messages.push(Message {
                role: MessageRole::User,
                content: input.to_message_content(),
            });
        }
        // updating data urls associated with the input
        self.data_urls.extend(input.data_urls());
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content: MessageContent::Text(output.to_string()),
        });
        // Clearing the session's role and marks the session as dirty (with unsaved changes)
        self.role = None;
        self.dirty = true;
        Ok(())
    }

    // this function echoes the messages in the session based on the given input
    pub fn echo_messages(&self, input: &Input) -> String {
        let messages = self.build_emssages(input);
        // Constructing and returning a yaml representation of the messages in the session
        serde_yaml::to_string(&messages).unwrap_or_else(|_| "Unable to echo message".into())
    }

    // this function builds and returns messages in the session based on the given input
    pub fn build_emssages(&self, input: &Input) -> Vec<Message> {
        // constructing messages if the session is empty or if it contains a role
        let mut messages = self.messages.clone();
        let mut need_add_msg = true;
        if messages.is_empty() {
            if let Some(role) = self.role.as_ref() {
                messages = role.build_messages(input);
                need_add_msg = false;
            }
        };
        if need_add_msg {
            messages.push(Message {
                role: MessageRole::User,
                content: input.to_message_content(),
            });
        }
        // Returning a vector of messages
        messages
    }
}
