mod input;
mod role;
mod session;

pub use self::input::Input;
use self::role::Role;
use self::session::{Session, TEMP_SESSION_NAME};

use crate::client::{
    create_client_config, list_client_types, list_models, ClientConfig, ExtraConfig, Message,
    Model, OpenAIClient, SendData,
};
use crate::render::{MarkdownRender, RenderOptions};
use crate::utils::{get_env_name, light_theme_from_colorfgbg, now, prompt_op_err, render_prompt};

use anyhow::{anyhow, bail, Context, Result};
use inquire::{Confirm, Select, Text};
use is_terminal::IsTerminal;
use parking_lot::RwLock;
use serde::Deserialize;
use std::collections::HashMap;
use std::{
    env,
    fs::{create_dir_all, read_dir, read_to_string, remove_file, File, OpenOptions},
    io::{stdout, Write},
    path::{Path, PathBuf},
    process::exit,
    sync::Arc,
};
use syntect::highlighting::ThemeSet;

/// Constants for Monokai Extended
const DARK_THEME: &[u8] = include_bytes!("../../assets/monokai-extended.theme.bin");
const LIGHT_THEME: &[u8] = include_bytes!("../../assets/monokai-extended-light.theme.bin");

const CONFIG_FILE_NAME: &str = "config.yaml";
const ROLES_FILE_NAME: &str = "roles.yaml";
const MESSAGES_FILE_NAME: &str = "messages.md";
const SESSIONS_DIR_NAME: &str = "sessions";

const CLIENTS_FIELD: &str = "clients";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// LLM model
    #[serde(rename(serialize = "model", deserialize = "model"))]
    pub model_id: Option<String>,
    /// GPT temperature, between 0 and 2
    #[serde(rename(serialize = "temperature", deserialize = "temperature"))]
    pub default_temperature: Option<f64>,
    /// Dry-run flag
    pub dry_run: bool,
    /// Whether to save the message
    pub save: bool,
    /// Whether to disable highlight
    pub highlight: bool,
    /// Whether to use a light theme
    pub light_theme: bool,
    /// Specify the text-wrapping mode (no, auto, <max-width>)
    pub wrap: Option<String>,
    /// Whether wrap code block
    pub wrap_code: bool,
    /// Automatically copy the last output to the clipboard
    pub auto_copy: bool,
    /// REPL keybindings. (emacs, vi)
    pub keybindings: Keybindings,
    /// Set a default role or session (role:<name>, session:<name>)
    pub prelude: String,
    /// REPL left prompt
    pub left_prompt: String,
    /// REPL right prompt
    pub right_prompt: String,
    /// Setup clients
    pub clients: Vec<ClientConfig>,
    /// Predefined roles
    #[serde(skip)]
    pub roles: Vec<Role>,
    /// Current selected role
    #[serde(skip)]
    pub role: Option<Role>,
    /// Current session
    #[serde(skip)]
    pub session: Option<Session>,
    #[serde(skip)]
    pub model: Model,
    #[serde(skip)]
    pub last_message: Option<(Input, String)>,
    #[serde(skip)]
    pub temperature: Option<f64>,
}

// here, we define the implementation of the Default trait for Config
impl Default for Config {
    fn default() -> Self {
        Self {
            model_id: None,
            default_temperature: None,
            save: true,
            highlight: true,
            dry_run: false,
            light_theme: false,
            wrap: None,
            wrap_code: false,
            auto_copy: false,
            keybindings: Default::default(),
            prelude: String::new(),
            left_prompt: "{color.green}{?session {session}{?role /}}{role}{color.cyan}{?session )}{!session >}{color.reset} ".to_string(),
            right_prompt: "{color.purple}{?session {?consume_tokens {consume_tokens}({consume_percent}%)}{!consume_tokens {consume_tokens}}}{color.reset}"
                .to_string(),
            clients: vec![ClientConfig::default()],
            roles: vec![],
            role: None,
            session: None,
            model: Default::default(),
            temperature: None,
            last_message: None,
        }
    }
}

// This defines a type alias GlobalConfig for a thread-safe, shared, mutable reference to a Config struct,
// wrapped in Arc (atomic reference counting) and RwLock (read-write lock)
// This allows multiple threads to access and modify the configuration concurrently
pub type GlobalConfig = Arc<RwLock<Config>>;

impl Config {
    // this function is responsible for initializing the application's configuration
    pub fn init(is_interactive: bool) -> Result<Self> {
        // getting the config_path using the config_file function from the the configuration
        let config_path = Self::config_file()?;

        // The openAI api key is retrieved from the environment variables
        let api_key = env::var("OPENAI_API_KEY").ok();

        let exist_config_path = config_path.exists();
        if is_interactive && api_key.is_none() && !exist_config_path {
            // we prompt to create a configuration file using the create_config_file method
            create_config_file(&config_path)?;
        }
        let mut config = if api_key.is_some() && !exist_config_path {
            Self::default()
        } else {
            Self::load_config(&config_path)?
        };

        // making compatible with old configuration files
        if exist_config_path {
            config.compat_old_config(&config_path)?;
        }

        if let Some(wrap) = config.wrap.clone() {
            config.set_wrap(&wrap)?;
        }

        // setting the temperature to the default temperture in the configuration
        config.temperature = config.default_temperature;

        config.load_roles()?;

        // setting upt the configurations of the model by calling some setter functions
        config.setup_model()?;
        config.setup_highlight();
        config.setup_light_theme()?;

        setup_logger()?;

        // returning the configurations wrapped in a Result
        Ok(config)
    }

    // function to be called at the start of the application
    pub fn onstart(&mut self) -> Result<()> {
        // checking the prelude configuration
        let prelude = self.prelude.clone();
        let err_msg = || format!("Invalid prelude '{}", prelude);
        match prelude.split_once(':') {
            // if the prelude contains "role"
            Some(("role", name)) => {
                // we set the role if it's not already set
                if self.role.is_none() && self.session.is_none() {
                    self.set_role(name).with_context(err_msg)?;
                }
            }
            // If the prelude contains "session"
            Some(("session", name)) => {
                // we start a new session if it's not already started
                if self.session.is_none() {
                    self.start_session(Some(name)).with_context(err_msg)?;
                }
            }
            // If the prelude is invalid
            Some(_) => {
                // we return an error
                bail!("{}", err_msg())
            }
            None => {}
        }
        // return if successful
        Ok(())
    }

    // function to retrieve a role by its name
    pub fn retrieve_role(&self, name: &str) -> Result<Role> {
        self.roles
            // iterates through the list of roles stored in the configuration
            .iter()
            // and finds the one with a matching name
            .find(|v| v.match_name(name))
            .map(|v| {
                // when matching role is found, we clone it
                let mut role = v.clone();
                // we complete its prompt arguments with the given name
                role.complete_prompt_args(name);
                // and return it
                role
            })
            .ok_or_else(|| anyhow!("Unknown role `{name}`"))
    }

    pub fn config_dir() -> Result<PathBuf> {
        let env_name = get_env_name("config_dir");
        let path = if let Some(v) = env::var_os(env_name) {
            PathBuf::from(v)
        } else {
            let mut dir = dirs::config_dir().ok_or_else(|| anyhow!("Not found config dir"))?;
            dir.push(env!("CARGO_CRATE_NAME"));
            dir
        };
        Ok(path)
    }

    // this function returns the directory path where the application's configuration files are stored
    pub fn local_path(name: &str) -> Result<PathBuf> {
        let mut path = Self::config_dir()?;
        path.push(name);
        Ok(path)
    }

    // this function is responsible for saving a message to a file or a session
    pub fn save_message(&mut self, input: Input, output: &str) -> Result<()> {
        // firstly, we update the last_message field with the input and output provided
        self.last_message = Some((input.clone(), output.to_string()));

        // if the dry_run flag is set
        if self.dry_run {
            // we return early without saving anything
            return Ok(());
        }

        // If a session is active
        if let Some(session) = self.session.as_mut() {
            //  we add the message to the session and return
            session.add_message(&input, output)?;
            return Ok(());
        }

        // saving is disabled (save is false), it returns early without saving
        if !self.save {
            return Ok(());
        }
        // else we write it in the file
        let mut file = self.open_message_file()?;
        if output.is_empty() || !self.save {
            return Ok(());
        }
        let timestamp = now();
        let input_markdown = input.render();
        let output = match self.role.as_ref() {
            None => {
                format!("# CHAT:[{timestamp}]\n{input_markdown}\n--------\n{output}\n--------\n\n",)
            }
            Some(v) => {
                format!(
                    "# CHAT:[{timestamp}] ({})\n{input_markdown}\n--------\n{output}\n--------\n\n",
                    v.name,
                )
            }
        };
        file.write_all(output.as_bytes())
            .with_context(|| "Failed to save message")
    }

    // this function returns the path to the configuration file (config.yaml)
    pub fn config_file() -> Result<PathBuf> {
        Self::local_path(CONFIG_FILE_NAME)
    }

    // this function returns the path to the roles file (roles.yaml)
    pub fn roles_file() -> Result<PathBuf> {
        let env_name = get_env_name("roles_file");
        env::var(env_name).map_or_else(
            |_| Self::local_path(ROLES_FILE_NAME),
            |value| Ok(PathBuf::from(value)),
        )
    }

    // this function returns the path to the messages file (messages.md)
    pub fn messages_file() -> Result<PathBuf> {
        Self::local_path(MESSAGES_FILE_NAME)
    }

    // this function returns the path to the directory where session files are stored (sessions)
    pub fn sessions_dir() -> Result<PathBuf> {
        Self::local_path(SESSIONS_DIR_NAME)
    }

    // This function constructs the path to a session file based on the session name
    pub fn session_file(name: &str) -> Result<PathBuf> {
        let mut path = Self::sessions_dir()?;
        path.push(&format!("{name}.yaml"));
        Ok(path)
    }

    // this function lets us, set the role for the current configuration based on the provided name
    pub fn set_role(&mut self, name: &str) -> Result<()> {
        let role = self.retrieve_role(name)?;
        if let Some(session) = self.session.as_mut() {
            session.update_role(Some(role.clone()))?;
        }
        self.temperature = role.temperature;
        self.role = Some(role);
        Ok(())
    }

    // this function is for clearing the current role from the configuration
    pub fn clear_role(&mut self) -> Result<()> {
        if let Some(session) = self.session.as_mut() {
            session.update_role(None)?;
        }
        self.temperature = self.default_temperature;
        self.role = None;
        Ok(())
    }

    // this function returns the current state of the configuration
    pub fn get_state(&self) -> State {
        if let Some(session) = &self.session {
            if session.is_empty() {
                if session.role.is_some() {
                    State::EmptySessionWithRole
                } else {
                    State::EmptySession
                }
            } else {
                State::Session
            }
        } else if self.role.is_some() {
            State::Role
        } else {
            State::Normal
        }
    }

    // this is a getter method for temperature
    pub fn get_temperature(&self) -> Option<f64> {
        self.temperature
    }

    // this function lets us set the temperature for the configuration
    pub fn set_temperature(&mut self, value: Option<f64>) -> Result<()> {
        self.temperature = value;
        if let Some(session) = self.session.as_mut() {
            session.set_temperature(value);
        }
        Ok(())
    }

    // this function echoes the messages based on the current configuration state
    pub fn echo_messages(&self, input: &Input) -> String {
        if let Some(session) = self.session.as_ref() {
            session.echo_messages(input)
        } else if let Some(role) = self.role.as_ref() {
            role.echo_messages(input)
        } else {
            input.render()
        }
    }

    // this function is for build messages based on the current configuration state
    pub fn build_messages(&self, input: &Input) -> Result<Vec<Message>> {
        // If a session is active, we build messages from the session
        let messages = if let Some(session) = self.session.as_ref() {
            session.build_emssages(input)
        } else if let Some(role) = self.role.as_ref() {
            role.build_messages(input)
        } else {
            let message = Message::new(input);
            vec![message]
        };
        Ok(messages)
    }

    // this function sets the "text wrapping mode" for the configuration
    pub fn set_wrap(&mut self, value: &str) -> Result<()> {
        if value == "no" {
            self.wrap = None;
        } else if value == "auto" {
            self.wrap = Some(value.into());
        } else {
            value
                .parse::<u16>()
                .map_err(|_| anyhow!("Invalid wrap value"))?;
            self.wrap = Some(value.into())
        }
        Ok(())
    }

    // this function is for setting the model for the configuration based on the provided value
    pub fn set_model(&mut self, value: &str) -> Result<()> {
        // retrieving a list of available models
        let models = list_models(self);
        let model = Model::find(&models, value);
        // attempting to find the model by matching
        match model {
            None => bail!("Invalid model '{}'", value),
            Some(model) => {
                if let Some(session) = self.session.as_mut() {
                    session.set_model(model.clone())?;
                }
                self.model = model;
                Ok(())
            }
        }
    }

    // this function generates system information for the configuration
    pub fn sys_info(&self) -> Result<String> {
        // this collects various configuration settings and paths,
        // such as model ID, temperature, file paths, and boolean settings
        let display_path = |path: &Path| path.display().to_string();
        let temperature = self
            .temperature
            .map_or_else(|| String::from("-"), |v| v.to_string());
        let wrap = self
            .wrap
            .clone()
            .map_or_else(|| String::from("no"), |v| v.to_string());
        let prelude = if self.prelude.is_empty() {
            String::from("-")
        } else {
            self.prelude.clone()
        };
        // this constructs a formatted string containing the configuration information
        let items = vec![
            ("model", self.model.id()),
            ("temperature", temperature),
            ("dry_run", self.dry_run.to_string()),
            ("save", self.save.to_string()),
            ("highlight", self.highlight.to_string()),
            ("light_theme", self.light_theme.to_string()),
            ("wrap", wrap),
            ("wrap_code", self.wrap_code.to_string()),
            ("auto_copy", self.auto_copy.to_string()),
            ("keybindings", self.keybindings.stringify().into()),
            ("prelude", prelude),
            ("config_file", display_path(&Self::config_file()?)),
            ("roles_file", display_path(&Self::roles_file()?)),
            ("messages_file", display_path(&Self::messages_file()?)),
            ("sessions_dir", display_path(&Self::sessions_dir()?)),
        ];
        let output = items
            .iter()
            .map(|(name, value)| format!("{name:<20}{value}"))
            .collect::<Vec<String>>()
            .join("\n");
        Ok(output)
    }

    // retrieves information about the current role in the configuration
    pub fn role_info(&self) -> Result<String> {
        if let Some(role) = &self.role {
            role.info()
        } else {
            bail!("No role")
        }
    }

    // This function returns information about the current session
    pub fn session_info(&self) -> Result<String> {
        // If a session exists
        if let Some(session) = &self.session {
            //  we render the session using a Markdown renderer
            let render_options = self.get_render_options()?;
            let mut markdown_render = MarkdownRender::init(render_options)?;
            //  return the result
            session.render(&mut markdown_render)
        } else {
            // if there is no session found, we return an error
            bail!("No session")
        }
    }

    // this function returns information about the current state
    pub fn info(&self) -> Result<String> {
        // If a session exists
        if let Some(session) = &self.session {
            // we return the exported information from the session
            session.export()
            // If no session exists but a role exists
        } else if let Some(role) = &self.role {
            // we return information about the role
            role.info()
        } else {
            // else, we return system information
            self.sys_info()
        }
    }

    // this function returns the last reply message
    pub fn last_reply(&self) -> &str {
        // If a last message exists
        self.last_message
            .as_ref()
            // returning the reply part of it
            .map(|(_, reply)| reply.as_str())
            .unwrap_or_default()
    }

    // this function provides auto-completion options for the REPL
    pub fn repl_complete(&self, cmd: &str, args: &[&str]) -> Vec<String> {
        let (values, filter) = if args.len() == 1 {
            let values = match cmd {
                ".role" => self.roles.iter().map(|v| v.name.clone()).collect(),
                ".model" => list_models(self).into_iter().map(|v| v.id()).collect(),
                ".session" => self.list_sessions(),
                ".set" => vec![
                    "temperature ",
                    "save ",
                    "highlight ",
                    "dry_run ",
                    "auto_copy ",
                ]
                .into_iter()
                .map(|v| v.to_string())
                .collect(),
                _ => vec![],
            };
            (values, args[0])
        } else if args.len() == 2 {
            let to_vec = |v: bool| vec![v.to_string()];
            let values = match args[0] {
                "save" => to_vec(!self.save),
                "highlight" => to_vec(!self.highlight),
                "dry_run" => to_vec(!self.dry_run),
                "auto_copy" => to_vec(!self.auto_copy),
                _ => vec![],
            };
            (values, args[1])
        } else {
            return vec![];
        };
        values
            .into_iter()
            .filter(|v| v.starts_with(filter))
            .collect()
    }

    // this function updates the state based on the provided data
    pub fn update(&mut self, data: &str) -> Result<()> {
        let parts: Vec<&str> = data.split_whitespace().collect();
        if parts.len() != 2 {
            // data must be in the format <key> <value>, else we return an error
            bail!("Usage: .set <key> <value>. If value is null, unset key.");
        }
        let key = parts[0];
        let value = parts[1];
        let unset = value == "null";
        // Depending on the key, we update different aspects of the state
        match key {
            // updating the temperature settings
            "temperature" => {
                let value = if unset {
                    None
                } else {
                    let value = value.parse().with_context(|| "Invalid value")?;
                    Some(value)
                };
                self.set_temperature(value)?;
            }
            // updating the save settings
            "save" => {
                let value = value.parse().with_context(|| "Invalid value")?;
                self.save = value;
            }
            // updating the highlight settings
            "highlight" => {
                let value = value.parse().with_context(|| "Invalid value")?;
                self.highlight = value;
            }
            // setting the dry run boolean
            "dry_run" => {
                let value = value.parse().with_context(|| "Invalid value")?;
                self.dry_run = value;
            }
            // updating the auto_copy setting
            "auto_copy" => {
                let value = value.parse().with_context(|| "Invalid value")?;
                self.auto_copy = value;
            }
            // for all else keys, we return an error with the key as unknown
            _ => bail!("Unknown key `{key}`"),
        }
        Ok(())
    }

    // this function is for starting a new session
    pub fn start_session(&mut self, session: Option<&str>) -> Result<()> {
        // If there is already a session running, we return an error
        if self.session.is_some() {
            bail!(
                "Already in a session, please run '.exit session' first to exit the current session."
            );
        }
        match session {
            // If no session name is given, we create a temporary session with a unique name
            None => {
                let session_file = Self::session_file(TEMP_SESSION_NAME)?;
                // trying to remove the old session information
                if session_file.exists() {
                    remove_file(session_file)
                        .with_context(|| "Failed to clean previous session")?;
                }
                // making the new session
                self.session = Some(Session::new(
                    TEMP_SESSION_NAME,
                    self.model.clone(),
                    self.role.clone(),
                ));
            }
            // If session name is given
            Some(name) => {
                //  we check if a session file exists for that name
                let session_path = Self::session_file(name)?;
                if !session_path.exists() {
                    // If the session file exists, we load the session from it
                    self.session = Some(Session::new(name, self.model.clone(), self.role.clone()));
                } else {
                    // else we just create a new session for that session name
                    let session = Session::load(name, &session_path)?;
                    let model = session.model().to_string();
                    self.temperature = session.temperature();
                    self.session = Some(session);
                    self.set_model(&model)?;
                }
            }
        }
        // session is empty and there is a last message
        if let Some(session) = self.session.as_mut() {
            if session.is_empty() {
                if let Some((input, output)) = &self.last_message {
                    // we ask the user, if to incorporate the last question and answer into the session
                    let ans = Confirm::new(
                        "Start a session that incorporates the last question and answer?",
                    )
                    .with_default(false)
                    .prompt()
                    .map_err(prompt_op_err)?;
                    if ans {
                        session.add_message(input, output)?;
                    }
                }
            }
        }
        // return
        Ok(())
    }

    // this function ends the current session
    pub fn end_session(&mut self) -> Result<()> {
        // if a session exists
        if let Some(mut session) = self.session.take() {
            // we clear the last message
            self.last_message = None;
            // reseting the temperature setting
            self.temperature = self.default_temperature;
            // Checking if the session should be saved
            if session.should_save() {
                // prompting user to confirm saving the session
                let ans = Confirm::new("Save session?")
                    .with_default(false)
                    .prompt()
                    .map_err(prompt_op_err)?;
                // if user says no, we return
                if !ans {
                    return Ok(());
                }
                // if user says yes
                let mut name = session.name().to_string();
                // we prompt for the session name (if it's temporary)
                if session.is_temp() {
                    name = Text::new("Session name:")
                        .with_default(&name)
                        .prompt()
                        .map_err(prompt_op_err)?;
                }
                let session_path = Self::session_file(&name)?;
                let sessions_dir = session_path.parent().ok_or_else(|| {
                    anyhow!("Unable to save session file to {}", session_path.display())
                })?;
                if !sessions_dir.exists() {
                    create_dir_all(sessions_dir).with_context(|| {
                        format!("Failed to create session_dir '{}'", sessions_dir.display())
                    })?;
                }
                //  save the session to a file
                session.save(&session_path)?;
            }
        }
        Ok(())
    }

    // This function is for listing all available sessions
    pub fn list_sessions(&self) -> Vec<String> {
        // Finding and reading the sessions directory
        let sessions_dir = match Self::sessions_dir() {
            Ok(dir) => dir,
            Err(_) => return vec![],
        };
        match read_dir(sessions_dir) {
            Ok(rd) => {
                //  extracting the names of session files
                let mut names = vec![];
                for entry in rd.flatten() {
                    let name = entry.file_name();
                    if let Some(name) = name.to_string_lossy().strip_suffix(".yaml") {
                        names.push(name.to_string());
                    }
                }
                names.sort_unstable();
                // returning all the names
                names
            }
            // if there is an error, we return and empty vector
            Err(_) => vec![],
        }
    }

    // this function determines the rendering options based on the current state
    pub fn get_render_options(&self) -> Result<RenderOptions> {
        // checking if highlighting is enabled
        let theme = if self.highlight {
            // Determine the theme mode
            let theme_mode = if self.light_theme { "light" } else { "dark" };
            let theme_filename = format!("{theme_mode}.tmTheme");
            let theme_path = Self::local_path(&theme_filename)?;
            if theme_path.exists() {
                // Attempts to load a theme file
                let theme = ThemeSet::get_theme(&theme_path)
                    .with_context(|| format!("Invalid theme at {}", theme_path.display()))?;
                Some(theme)
            } else {
                // if theme path doesn't exist, we check for the given theme
                let theme = if self.light_theme {
                    bincode::deserialize_from(LIGHT_THEME).expect("Invalid builtin light theme")
                } else {
                    bincode::deserialize_from(DARK_THEME).expect("Invalid builtin dark theme")
                };
                // return the theme wrapped in a Result
                Some(theme)
            }
        } else {
            // If no highlight is given, we return None
            None
        };
        let wrap = if stdout().is_terminal() {
            self.wrap.clone()
        } else {
            None
        };
        // constructing and returning RenderOptions with the determined theme, wrap option
        Ok(RenderOptions::new(theme, wrap, self.wrap_code))
    }

    // this function generates the left part of the prompt based on templates and current context
    pub fn render_prompt_left(&self) -> String {
        // generate a context (Hashmap)
        let variables = self.generate_prompt_context();
        // render the left prompt using the makde context
        render_prompt(&self.left_prompt, &variables)
    }

    // this function generates the right part of the prompt based on templates and current context
    pub fn render_prompt_right(&self) -> String {
        // generate a context (Hashmap)
        let variables = self.generate_prompt_context();
        // render the left prompt using the makde context
        render_prompt(&self.right_prompt, &variables)
    }

    // this function prepares data based on the input, different based on whether the operation should be streamed or not
    pub fn prepare_send_data(&self, input: &Input, stream: bool) -> Result<SendData> {
        // building messages from the input
        let messages = self.build_messages(input)?;
        // we check if the total tokens of the messages exceed the model's limit
        self.model.max_tokens_limit(&messages)?;
        // return the built messages in SendData method
        Ok(SendData {
            messages,
            temperature: self.get_temperature(),
            stream,
        })
    }

    // this function calculates and prints the total token count of the input without actually sending it
    pub fn maybe_print_send_tokens(&self, input: &Input) {
        if self.dry_run {
            // building messages from the input
            if let Ok(messages) = self.build_messages(input) {
                // get the max tokens
                let tokens = self.model.total_tokens(&messages);
                // Print the token count
                println!(">>> This message consumes {tokens} tokens. <<<");
            }
        }
    }

    // this function generates a context for rendering prompts
    fn generate_prompt_context(&self) -> HashMap<&str, String> {
        // a HashMap for storing the key-value pairs representing various settings and states
        let mut output = HashMap::new();
        // inserting various key-value pairs in the hashmap
        output.insert("model", self.model.id());
        output.insert("client_name", self.model.client_name.clone());
        output.insert("model_name", self.model.name.clone());
        output.insert(
            "max_tokens",
            self.model.max_tokens.unwrap_or_default().to_string(),
        );
        if let Some(temperature) = self.temperature {
            if temperature != 0.0 {
                output.insert("temperature", temperature.to_string());
            }
        }
        if self.dry_run {
            output.insert("dry_run", "true".to_string());
        }
        if self.save {
            output.insert("save", "true".to_string());
        }
        if let Some(wrap) = &self.wrap {
            if wrap != "no" {
                output.insert("wrap", wrap.clone());
            }
        }
        if self.auto_copy {
            output.insert("auto_copy", "true".to_string());
        }
        if let Some(role) = &self.role {
            output.insert("role", role.name.clone());
        }
        if let Some(session) = &self.session {
            output.insert("session", session.name().to_string());
            let (tokens, percent) = session.tokens_and_percent();
            output.insert("consume_tokens", tokens.to_string());
            output.insert("consume_percent", percent.to_string());
            output.insert("user_messages_len", session.user_messages_len().to_string());
        }

        // highlighting is enabled, we add ANSI color codes to the context
        if self.highlight {
            output.insert("color.reset", "\u{1b}[0m".to_string());
            output.insert("color.black", "\u{1b}[30m".to_string());
            output.insert("color.dark_gray", "\u{1b}[90m".to_string());
            output.insert("color.red", "\u{1b}[31m".to_string());
            output.insert("color.light_red", "\u{1b}[91m".to_string());
            output.insert("color.green", "\u{1b}[32m".to_string());
            output.insert("color.light_green", "\u{1b}[92m".to_string());
            output.insert("color.yellow", "\u{1b}[33m".to_string());
            output.insert("color.light_yellow", "\u{1b}[93m".to_string());
            output.insert("color.blue", "\u{1b}[34m".to_string());
            output.insert("color.light_blue", "\u{1b}[94m".to_string());
            output.insert("color.purple", "\u{1b}[35m".to_string());
            output.insert("color.light_purple", "\u{1b}[95m".to_string());
            output.insert("color.magenta", "\u{1b}[35m".to_string());
            output.insert("color.light_magenta", "\u{1b}[95m".to_string());
            output.insert("color.cyan", "\u{1b}[36m".to_string());
            output.insert("color.light_cyan", "\u{1b}[96m".to_string());
            output.insert("color.white", "\u{1b}[37m".to_string());
            output.insert("color.light_gray", "\u{1b}[97m".to_string());
        }

        // returning the output
        output
    }

    // this function opens the message file for appending messages
    fn open_message_file(&self) -> Result<File> {
        // getting the message file path
        let path = Self::messages_file()?;
        // ensuring the path exists
        ensure_parent_exists(&path)?;
        // we open the file in append mode, and/or create it if it doesn't exist
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to create/append {}", path.display()))
    }

    // this function loads the configuration from a yaml file located at the specified config_path
    fn load_config(config_path: &Path) -> Result<Self> {
        // reading the content of the file
        let ctx = || format!("Failed to load config at {}", config_path.display());
        let content = read_to_string(config_path).with_context(ctx)?;

        // deserializing context into Config
        let config: Self = serde_yaml::from_str(&content)
            .map_err(|err| {
                let err_msg = err.to_string();
                if err_msg.starts_with(&format!("{}: ", CLIENTS_FIELD)) {
                    anyhow!("clients: invalid value")
                } else {
                    anyhow!("{err_msg}")
                }
            })
            .with_context(ctx)?;

        // we return the resulting configuration
        Ok(config)
    }

    // this function loads roles from a yaml file and sets the roles field of the struct
    fn load_roles(&mut self) -> Result<()> {
        // get the path to the roles file
        let path = Self::roles_file()?;
        // if path does not exist, we return
        if !path.exists() {
            return Ok(());
        }
        // read the content of the file
        let content = read_to_string(&path)
            .with_context(|| format!("Failed to load roles at {}", path.display()))?;
        // deserialize it into a vector of Role structs
        let roles: Vec<Role> =
            serde_yaml::from_str(&content).with_context(|| "Invalid roles config")?;
        self.roles = roles;
        Ok(())
    }

    // This function sets up the model using the provided model ID or selecting the first available model
    fn setup_model(&mut self) -> Result<()> {
        let model = match &self.model_id {
            // some model is found with the given id
            Some(v) => v.clone(),
            None => {
                // If no models are available, we return an error
                let models = list_models(self);
                if models.is_empty() {
                    bail!("No available model");
                }

                // return the model id
                models[0].id()
            }
        };
        self.set_model(&model)?;
        Ok(())
    }

    // This function checks if the NO_COLOR environment variable is set and sets highlight according to it
    fn setup_highlight(&mut self) {
        // getting the value of the NO_COLOR variable
        if let Ok(value) = env::var("NO_COLOR") {
            // delcaring another variable for no_color
            let mut no_color = false;
            // we set the new variable to the value in configuration
            set_bool(&mut no_color, &value);
            // if no_color is true, we make the highlight false
            if no_color {
                self.highlight = false;
            }
        }
    }

    // this function sets up the light theme based on environment variables
    fn setup_light_theme(&mut self) -> Result<()> {
        // checking if the light_theme field is already set to true
        if self.light_theme {
            return Ok(());
        }
        // checking environment variables for configuration
        if let Ok(value) = env::var(get_env_name("light_theme")) {
            set_bool(&mut self.light_theme, &value);
            return Ok(());
        }
        // if light_theme is not set and environment variables are not found
        else if let Ok(value) = env::var("COLORFGBG") {
            // we determine  the light theme based on the COLORFGBG environment variable
            if let Some(light) = light_theme_from_colorfgbg(&value) {
                self.light_theme = light
            }
        };
        Ok(())
    }

    // this function ensures compatibility with old configurations with the new one
    fn compat_old_config(&mut self, config_path: &PathBuf) -> Result<()> {
        // reading the content of the configuration file
        let content = read_to_string(config_path)?;
        // Parsing the yaml into a json value
        let value: serde_json::Value = serde_yaml::from_str(&content)?;
        // checking if configuration already contains a field named CLIENTS_FIELD, impling that configuration is already in the new format
        if value.get(CLIENTS_FIELD).is_some() {
            return Ok(());
        }

        // Retrieving the value of the "model" field from the config
        if let Some(model_name) = value.get("model").and_then(|v| v.as_str()) {
            // model name starts with "gpt", we set the model_id field in the struct as per new format
            if model_name.starts_with("gpt") {
                self.model_id = Some(format!("{}:{}", OpenAIClient::NAME, model_name));
            }
        }

        // retrieving the first client configuration from clients in struct
        if let Some(ClientConfig::OpenAIConfig(client_config)) = self.clients.get_mut(0) {
            // and then we update various fields of the client configurations
            if let Some(api_key) = value.get("api_key").and_then(|v| v.as_str()) {
                client_config.api_key = Some(api_key.to_string())
            }

            if let Some(organization_id) = value.get("organization_id").and_then(|v| v.as_str()) {
                client_config.organization_id = Some(organization_id.to_string())
            }

            let mut extra_config = ExtraConfig::default();

            if let Some(proxy) = value.get("proxy").and_then(|v| v.as_str()) {
                extra_config.proxy = Some(proxy.to_string())
            }

            if let Some(connect_timeout) = value.get("connect_timeout").and_then(|v| v.as_i64()) {
                extra_config.connect_timeout = Some(connect_timeout as _)
            }

            client_config.extra = Some(extra_config);
        }
        Ok(())
    }
}

// This enum represents different keybinding modes (i.e. Emacs or Vim)
#[derive(Debug, Clone, Deserialize, Default)]
pub enum Keybindings {
    #[serde(rename = "emacs")]
    #[default]
    Emacs,
    #[serde(rename = "vi")]
    Vi,
}

// this block implements methods to check if the keybinding is Vi and to get string of the enum
impl Keybindings {
    pub fn is_vi(&self) -> bool {
        matches!(self, Keybindings::Vi)
    }
    pub fn stringify(&self) -> &str {
        match self {
            Keybindings::Emacs => "emacs",
            Keybindings::Vi => "vi",
        }
    }
}

// This enum represents different states of the application
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum State {
    Normal,
    Role,
    EmptySession,
    EmptySessionWithRole,
    Session,
}

// Below are some util function for this file

// this function prompts the user to create a new configuration file if it doesn't exist
fn create_config_file(config_path: &Path) -> Result<()> {
    let ans = Confirm::new("No config file, create a new one?")
        .with_default(true)
        .prompt()
        .map_err(prompt_op_err)?;
    if !ans {
        exit(0);
    }

    let client = Select::new("Platform:", list_client_types())
        .prompt()
        .map_err(prompt_op_err)?;

    let mut config = serde_json::json!({});
    config["model"] = client.into();
    config[CLIENTS_FIELD] = create_client_config(client)?;

    let config_data = serde_yaml::to_string(&config).with_context(|| "Failed to create config")?;

    ensure_parent_exists(config_path)?;
    std::fs::write(config_path, config_data).with_context(|| "Failed to write to config file")?;
    #[cfg(unix)]
    {
        use std::os::unix::prelude::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(config_path, perms)?;
    }

    println!("✨ Saved config file to {}\n", config_path.display());

    Ok(())
}

// This function ensures that the parent directory of a given file path exists
fn ensure_parent_exists(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Failed to write to {}, No parent path", path.display()))?;
    if !parent.exists() {
        create_dir_all(parent).with_context(|| {
            format!(
                "Failed to write {}, Cannot create parent directory",
                path.display()
            )
        })?;
    }
    Ok(())
}

// This function sets a boolean variable based on a string value
fn set_bool(target: &mut bool, value: &str) {
    match value {
        "1" | "true" => *target = true,
        "0" | "false" => *target = false,
        _ => {}
    }
}

// Below are the functions which configure logging based on whether the application is in debug mode or not
#[cfg(debug_assertions)]
// in this function logging set up to write debug logs to a file named "debug.log".
fn setup_logger() -> Result<()> {
    use simplelog::{LevelFilter, WriteLogger};
    let file = std::fs::File::create(Config::local_path("debug.log")?)?;
    let log_filter = match std::env::var("AICHAT_LOG_FILTER") {
        Ok(v) => v,
        Err(_) => "aichat".into(),
    };
    let config = simplelog::ConfigBuilder::new()
        .add_filter_allow(log_filter)
        .set_thread_level(LevelFilter::Off)
        .set_time_level(LevelFilter::Off)
        .build();
    WriteLogger::init(log::LevelFilter::Debug, config, file)?;
    Ok(())
}

#[cfg(not(debug_assertions))]
fn setup_logger() -> Result<()> {
    Ok(())
}
