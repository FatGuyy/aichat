use clap::Parser;
// This file uses clap crate for parsing and handling command-line arguments

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    // Choose a LLM model
    #[clap(short, long)]
    pub model: Option<String>,
    // Choose a role
    #[clap(short, long)]
    pub role: Option<String>,
    // Create or reuse a session
    #[clap(short = 's', long)]
    pub session: Option<Option<String>>,
    // Attach files to the message to be sent.
    #[clap(short = 'f', long, num_args = 1.., value_name = "FILE")]
    pub file: Option<Vec<String>>,
    // Disable syntax highlighting
    #[clap(short = 'H', long)]
    pub no_highlight: bool,
    // No stream output
    #[clap(short = 'S', long)]
    pub no_stream: bool,
    // Specify the text-wrapping mode (no, auto, <max-width>)
    #[clap(short = 'w', long)]
    pub wrap: Option<String>,
    // Use light theme
    #[clap(long)]
    pub light_theme: bool,
    // Run in dry run mode
    #[clap(long)]
    pub dry_run: bool,
    // Print related information
    #[clap(long)]
    pub info: bool,
    // List all available models
    #[clap(long)]
    pub list_models: bool,
    // List all available roles
    #[clap(long)]
    pub list_roles: bool,
    // List all available sessions
    #[clap(long)]
    pub list_sessions: bool,
    // Input text
    text: Vec<String>,
}

impl Cli {
    // This method processes text input from the command-line interface
    // by trimming whitespace and joining the individual strings
    pub fn text(&self) -> Option<String> {
        let text = self
            .text
            .iter() // Iterates over the text field of Cli struct
            .map(|x| x.trim().to_string()) // trim leading and trailing whitespaces
            .collect::<Vec<String>>() // Collect the new string into a vector
            .join(" "); // join the vector if strings, so that we have every string with a space inbetween

        if text.is_empty() {
            // If the resulting string is empty, return None
            return None;
        }
        Some(text) // Else return string wrapped in 'Some'
    }
}
