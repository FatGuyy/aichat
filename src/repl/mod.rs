mod completer;
mod highlighter;
mod prompt;

use self::completer::ReplCompleter;
use self::highlighter::ReplHighlighter;
use self::prompt::ReplPrompt;

use crate::client::{ensure_model_capabilities, init_client};
use crate::config::{GlobalConfig, Input, State};
use crate::render::{render_error, render_stream};
use crate::utils::{create_abort_signal, set_text, AbortSignal};

use anyhow::{bail, Context, Result};
use fancy_regex::Regex;
use lazy_static::lazy_static;
use reedline::Signal;
use reedline::{
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    ColumnarMenu, EditMode, Emacs, KeyCode, KeyModifiers, Keybindings, Reedline, ReedlineEvent,
    ReedlineMenu, ValidationResult, Validator, Vi,
};

// constant string for storing completion_menu
const MENU_NAME: &str = "completion_menu";

// lazily initialized static array of ReplCommand, for representing a command that can be executed within the REPL
lazy_static! {
    static ref REPL_COMMANDS: [ReplCommand; 13] = [
        // Commands are .help; .info; .model; .role
        // the things the commands perform are written in front of them
        ReplCommand::new(".help", "Print this help message", vec![]),
        ReplCommand::new(".info", "Print system info", vec![]),
        ReplCommand::new(".model", "Switch LLM model", vec![]),
        ReplCommand::new(".role", "Use a role", vec![State::Session]),
        ReplCommand::new(
            ".info role", // another command
            "Show role info",
            vec![State::Normal, State::EmptySession, State::Session]
        ),
        ReplCommand::new(
            ".exit role", // another command
            "Leave current role",
            vec![State::Normal, State::EmptySession, State::Session]
        ),
        ReplCommand::new(
            ".session", // another command
            "Start a context-aware chat session",
            vec![
                State::EmptySession,
                State::EmptySessionWithRole,
                State::Session
            ]
        ),
        ReplCommand::new(
            ".info session", // another command
            "Show session info",
            vec![State::Normal, State::Role]
        ),
        ReplCommand::new(
            ".exit session", // another command
            "End the current session",
            vec![State::Normal, State::Role]
        ),
        ReplCommand::new(
            ".file", // another command
            "Attach files to the message and then submit it",
            vec![]
        ),
        // few more commands
        ReplCommand::new(".set", "Modify the configuration parameters", vec![]),
        ReplCommand::new(".copy", "Copy the last reply to the clipboard", vec![]),
        ReplCommand::new(".exit", "Exit the REPL", vec![]),
    ];
    // a regex instance for matching commands (prefixed with a dot and followed by non-space characters)
    static ref COMMAND_RE: Regex = Regex::new(r"^\s*(\.\S*)\s*").unwrap();
    // a regex instance to capture multiline input enclosed within ::: markers
    static ref MULTILINE_RE: Regex = Regex::new(r"(?s)^\s*:::\s*(.*)\s*:::\s*$").unwrap();
}

// This struct is for maanging the behavior of the REPL
pub struct Repl {
    config: GlobalConfig,
    editor: Reedline,
    prompt: ReplPrompt,
    abort: AbortSignal,
}

impl Repl {
    // this function initializes a repl instance with the given configurations
    pub fn init(config: &GlobalConfig) -> Result<Self> {
        // setting up the editor
        let editor = Self::create_editor(config)?;

        // Iinitialize prompt and abort signal
        let prompt = ReplPrompt::new(config);
        let abort = create_abort_signal();

        // return self, with config and above made variables
        Ok(Self {
            config: config.clone(),
            editor,
            prompt,
            abort,
        })
    }

    // this function has the main loop of REPL, for handling user input and system signals
    pub fn run(&mut self) -> Result<()> {
        // Display a banner
        self.banner();

        let mut already_ctrlc = false;

        // entering a loop where it waits for input from the user or a signal (Ctrl+C or Ctrl+D)
        loop {
            if self.abort.aborted_ctrld() {
                break;
            }
            if self.abort.aborted_ctrlc() && !already_ctrlc {
                already_ctrlc = true;
            }
            // reading the input from the user
            let sig = self.editor.read_line(&self.prompt);
            // match the imput with all the commands
            match sig {
                // if it is a valid command we run this
                Ok(Signal::Success(line)) => {
                    // set the ctrlc to false and reset the abort signal
                    already_ctrlc = false;
                    self.abort.reset();
                    // match the "line" by calling the handle function
                    match self.handle(&line) {
                        // if the message is Ok(quit) and quit is true
                        Ok(quit) => {
                            if quit {
                                // we break out of the loop
                                break;
                            }
                        }
                        // if it returns an error
                        Err(err) => {
                            // we output the error and with proper highlights
                            render_error(err, self.config.read().highlight);
                            println!()
                        }
                    }
                }
                // if the command is not received and a signal of crtlC is given
                Ok(Signal::CtrlC) => {
                    // set the ctrlc
                    self.abort.set_ctrlc();
                    if already_ctrlc {
                        // break out of the loop
                        break;
                    }
                    // set the already_ctrlc flag to true, so we can exit on the next press of ctrlC
                    already_ctrlc = true;
                    println!("(To exit, press Ctrl+C again or Ctrl+D or type .exit)\n");
                }
                // if the command is not received and a signal of crtlD is given
                Ok(Signal::CtrlD) => {
                    self.abort.set_ctrld();
                    // break out of the loop
                    break;
                }
                _ => {}
            }
        }
        // lastly, we explicitly call ".exit session" to properly close any ongoing sessions
        self.handle(".exit session")?;
        Ok(())
    }

    // this function is responsible for interpreting and executing commands entered by the user
    fn handle(&self, mut line: &str) -> Result<bool> {
        // checking if the user input matches the multiline input pattern defined by MULTILINE_RE constant
        if let Ok(Some(captures)) = MULTILINE_RE.captures(line) {
            // extracting the actual content to be processed
            if let Some(text_match) = captures.get(1) {
                line = text_match.as_str();
            }
        }
        // use parse_command function to get the command from the imput line
        match parse_command(line) {
            // Some((cmd, args)) is returned by the parse_command function
            // we match the cmd with the valid commands
            Some((cmd, args)) => match cmd {
                ".help" => {
                    // this calls dump_repl_help, which displays a help message with available commands
                    dump_repl_help();
                }
                // if  the command is info, we check the args
                ".info" => match args {
                    // depending on the args, we display information about the role, session, or system
                    Some("role") => {
                        let info = self.config.read().role_info()?;
                        println!("{}", info);
                    }
                    Some("session") => {
                        let info = self.config.read().session_info()?;
                        println!("{}", info);
                    }
                    Some(_) => unknown_command()?,
                    None => {
                        // if args are none, we just return the system info
                        let output = self.config.read().sys_info()?;
                        println!("{}", output);
                    }
                },
                // this is a Deprecated command, which suggests the user to use ::: for multiline inputs
                ".edit" => {
                    println!(r#"Deprecated. Use ::: instead."#);
                }
                // this cmd sets the LLM to a specified model name, if provided
                ".model" => match args {
                    Some(name) => {
                        self.config.write().set_model(name)?;
                    }
                    // if no args are given, we prompt the usage
                    None => println!("Usage: .model <name>"),
                },
                // this allows users to set or change the role
                ".role" => match args {
                    // it has args that are associated with the role change
                    Some(args) => match args.split_once(|c| c == '\n' || c == ' ') {
                        Some((name, text)) => {
                            let name = name.trim();
                            let text = text.trim();
                            let old_role =
                                self.config.read().role.as_ref().map(|v| v.name.to_string());
                            self.config.write().set_role(name)?;
                            self.ask(text, vec![])?;
                            match old_role {
                                Some(old_role) => self.config.write().set_role(&old_role)?,
                                None => self.config.write().clear_role()?,
                            }
                        }
                        None => {
                            self.config.write().set_role(args)?;
                        }
                    },
                    // if no args are provided, we prompt this to the user
                    None => println!(r#"Usage: .role <name> [text...]"#),
                },
                // this starts a session with optional arguments
                ".session" => {
                    self.config.write().start_session(args)?;
                }
                // this updates config parameters with the provided arguments
                ".set" => {
                    if let Some(args) = args {
                        self.config.write().update(args)?;
                    }
                }
                // this copies the last reply to the clipboard
                ".copy" => {
                    let config = self.config.read();
                    self.copy(config.last_reply())
                        .with_context(|| "Failed to copy the last output")?;
                }
                // this is a Deprecated command, suggesting the use of .file instead
                ".read" => {
                    println!(r#"Deprecated. Use '.file' instead."#);
                }
                // this attaches files and optionally additional text to the message
                ".file" => match args {
                    Some(args) => {
                        let (files, text) = match args.split_once(" -- ") {
                            Some((files, text)) => (files.trim(), text.trim()),
                            None => (args, ""),
                        };
                        let files = shell_words::split(files).with_context(|| "Invalid args")?;
                        self.ask(text, files)?;
                    }
                    None => println!("Usage: .file <files>...[ -- <text>...]"),
                },
                // this handles exiting from roles, sessions, or the REPL itself based on the arguments
                ".exit" => match args {
                    Some("role") => {
                        self.config.write().clear_role()?;
                    }
                    Some("session") => {
                        self.config.write().end_session()?;
                    }
                    Some(_) => unknown_command()?,
                    None => {
                        return Ok(true);
                    }
                },
                // this is a deprecated this command, advising the correct commands to use instead
                ".clear" => match args {
                    Some("role") => {
                        println!(r#"Deprecated. Use ".exit role" instead."#);
                    }
                    Some("conversation") => {
                        println!(r#"Deprecated. Use ".exit session" instead."#);
                    }
                    _ => unknown_command()?,
                },
                _ => unknown_command()?,
            },
            None => {
                self.ask(line, vec![])?;
            }
        }

        println!();

        Ok(false)
    }

    // this function handles the sending of user input to an AI model
    fn ask(&self, text: &str, files: Vec<String>) -> Result<()> {
        // if both text and files are empty, we immediately return, that there's nothing to process
        if text.is_empty() && files.is_empty() {
            return Ok(());
        }
        //
        let input = if files.is_empty() {
            // If there are no files, we simply use the text
            Input::from_str(text)
        } else {
            // otherwise, we create a new Input instance from both text and files
            Input::new(text, files)?
        };
        // printing the tokens of the input if configured to do so
        self.config.read().maybe_print_send_tokens(&input);
        // making new client
        let mut client = init_client(&self.config)?;
        ensure_model_capabilities(client.as_mut(), input.required_capabilities())?;
        let output = render_stream(&input, client.as_ref(), &self.config, self.abort.clone())?;
        self.config.write().save_message(input, &output)?;
        if self.config.read().auto_copy {
            let _ = self.copy(&output);
        }
        Ok(())
    }

    // this function is for displaying a welcome banner to the user when the REPL starts
    fn banner(&self) {
        let version = env!("CARGO_PKG_VERSION");
        print!(
            r#"Welcome to aichat {version}
Type ".help" for more information.
"#
        )
    }

    // this function initializes and configures the Reedline editor for user input
    fn create_editor(config: &GlobalConfig) -> Result<Reedline> {
        // initializing a completer, highlighter, configuring a menu and the edit mode for the editor
        let completer = ReplCompleter::new(config);
        let highlighter = ReplHighlighter::new(config);
        let menu = Self::create_menu();
        let edit_mode = Self::create_edit_mode(config);
        // we finally create a new Reedline editor using the above configurations
        let editor = Reedline::create()
            .with_completer(Box::new(completer))
            .with_highlighter(Box::new(highlighter))
            .with_menu(menu)
            .with_edit_mode(edit_mode)
            .with_quick_completions(true)
            .with_partial_completions(true)
            .use_bracketed_paste(true)
            .with_validator(Box::new(ReplValidator))
            .with_ansi_colors(true);

        // returning the editor wrapped in result
        Ok(editor)
    }

    // this function adds additional keybindings to the editor
    fn extra_keybindings(keybindings: &mut Keybindings) {
        keybindings.add_binding(
            KeyModifiers::NONE,
            KeyCode::Tab,
            ReedlineEvent::UntilFound(vec![
                ReedlineEvent::Menu(MENU_NAME.to_string()),
                ReedlineEvent::MenuNext,
            ]),
        );
        keybindings.add_binding(
            KeyModifiers::SHIFT,
            KeyCode::BackTab,
            ReedlineEvent::MenuPrevious,
        );
    }

    // this function determines and configures the edit mode for the editor based on the user configuration
    fn create_edit_mode(config: &GlobalConfig) -> Box<dyn EditMode> {
        // Checking the user configuration to decide between Emacs and Vi edit modes
        let edit_mode: Box<dyn EditMode> = if config.read().keybindings.is_vi() {
            let mut normal_keybindings = default_vi_normal_keybindings();
            let mut insert_keybindings = default_vi_insert_keybindings();
            // adding extra keybindings to the chosen edit mode's default keybindings
            Self::extra_keybindings(&mut normal_keybindings);
            Self::extra_keybindings(&mut insert_keybindings);
            Box::new(Vi::new(insert_keybindings, normal_keybindings))
        } else {
            let mut keybindings = default_emacs_keybindings();
            Self::extra_keybindings(&mut keybindings);
            Box::new(Emacs::new(keybindings))
        };
        // returning the configured edit mode
        edit_mode
    }

    // this function creates a Reedline menu
    fn create_menu() -> ReedlineMenu {
        // making the completetion menu with the constant MENU_NAME
        let completion_menu = ColumnarMenu::default().with_name(MENU_NAME);
        // returning the menu
        ReedlineMenu::EngineCompleter(Box::new(completion_menu))
    }

    // this function just makes the copy of a given text
    fn copy(&self, text: &str) -> Result<()> {
        // if the input is empty, we throw an error
        if text.is_empty() {
            bail!("No text")
        }
        // else, we give it inside set_text function
        set_text(text)?;
        // return
        Ok(())
    }
}

// this struct represents all the ReplCommands
#[derive(Debug, Clone)]
pub struct ReplCommand {
    name: &'static str,             // this is the name of the commnad
    description: &'static str,      // this is the description of what the command does
    unavailable_states: Vec<State>, // this is the availablility of the course
}

impl ReplCommand {
    // this is a constructor for the ReplCommnad struct
    fn new(name: &'static str, desc: &'static str, unavailable_states: Vec<State>) -> Self {
        Self {
            name,
            description: desc,
            unavailable_states,
        }
    }

    // this funciton returns true if the command is unavailable
    fn unavailable(&self, state: &State) -> bool {
        self.unavailable_states.contains(state)
    }
}

// A default validator which checks for mismatched quotes and brackets
struct ReplValidator;

// here we implement the Validator trait for the ReplValidator
impl Validator for ReplValidator {
    // this function validates the imput of the user and checks if it is valid
    fn validate(&self, line: &str) -> ValidationResult {
        let line = line.trim();
        if line.starts_with(r#":::"#) && !line[3..].ends_with(r#":::"#) {
            ValidationResult::Incomplete
        } else {
            ValidationResult::Complete
        }
    }
}

// this function is called when we need to throw an error saying the command is unknown
fn unknown_command() -> Result<()> {
    bail!(r#"Unknown command. Type ".help" for more information."#);
}

fn dump_repl_help() {
    let head = REPL_COMMANDS
        .iter()
        .map(|cmd| format!("{:<24} {}", cmd.name, cmd.description))
        .collect::<Vec<String>>()
        .join("\n");
    println!(
        r###"{head}

Type ::: to begin multi-line editing, type ::: to end it.
Press Ctrl+C to abort aichat, Ctrl+D to exit the REPL"###,
    );
}

// this function is to analyze a line of input text and determine if it starts with a command
fn parse_command(line: &str) -> Option<(&str, Option<&str>)> {
    // using the COMMAND_RE to match the input line
    // command = that start with a dot followed by one or more non-space characters (
    match COMMAND_RE.captures(line) {
        Ok(Some(captures)) => {
            let cmd = captures.get(1)?.as_str();
            let args = line[captures[0].len()..].trim();
            let args = if args.is_empty() { None } else { Some(args) };
            Some((cmd, args))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_command_line() {
        assert_eq!(parse_command(" ."), Some((".", None)));
        assert_eq!(parse_command(" .role"), Some((".role", None)));
        assert_eq!(parse_command(" .role  "), Some((".role", None)));
        assert_eq!(
            parse_command(" .set dry_run true"),
            Some((".set", Some("dry_run true")))
        );
        assert_eq!(
            parse_command(" .set dry_run true  "),
            Some((".set", Some("dry_run true")))
        );
        assert_eq!(
            parse_command(".prompt \nabc\n"),
            Some((".prompt", Some("abc")))
        );
    }
}
