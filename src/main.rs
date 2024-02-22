mod cli;
mod client;
mod config;
mod render;
mod repl;

#[macro_use]
extern crate log;
#[macro_use]
mod utils;

use crate::cli::Cli;
use crate::config::{Config, GlobalConfig};

use anyhow::Result;
// We are using clap for parsing command-line arguments
use clap::Parser;
use client::{ensure_model_capabilities, init_client, list_models};
use config::Input;
use is_terminal::IsTerminal;
use parking_lot::RwLock;
use render::{render_error, render_stream, MarkdownRender};
use repl::Repl;
use std::io::{stderr, stdin, stdout, Read};
use std::sync::Arc;
use utils::{cl100k_base_singleton, create_abort_signal};

// This is the main entry point for our porgram
// we Initialize various configurations and handle
// the command line arguments like models, sessions, themes
fn main() -> Result<()> {
    // initializing required variables and objects
    let cli = Cli::parse();
    let text = cli.text();
    // making the config variable, for storing all the required configurations
    let config = Arc::new(RwLock::new(Config::init(text.is_none())?));
    if cli.list_roles {
        config
            .read()
            .roles
            .iter()
            .for_each(|v| println!("{}", v.name));
        return Ok(());
    }
    if cli.list_models {
        for model in list_models(&config.read()) {
            println!("{}", model.id());
        }
        return Ok(());
    }
    if cli.list_sessions {
        let sessions = config.read().list_sessions().join("\n");
        println!("{sessions}");
        return Ok(());
    }
    if let Some(wrap) = &cli.wrap {
        config.write().set_wrap(wrap)?;
    }
    if cli.light_theme {
        config.write().light_theme = true;
    }
    if cli.dry_run {
        config.write().dry_run = true;
    }
    if let Some(name) = &cli.role {
        config.write().set_role(name)?;
    }
    if let Some(session) = &cli.session {
        config
            .write()
            .start_session(session.as_ref().map(|v| v.as_str()))?;
    }
    if let Some(model) = &cli.model {
        config.write().set_model(model)?;
    }
    if cli.no_highlight {
        config.write().highlight = false;
    }
    if cli.info {
        let info = config.read().info()?;
        println!("{}", info);
        return Ok(());
    }
    config.write().onstart()?;
    // Here after initializing all the arguments, we call the start function to begin the processing the request
    if let Err(err) = start(&config, text, cli.file, cli.no_stream) {
        let highlight = stderr().is_terminal() && config.read().highlight;
        render_error(err, highlight)
    }
    Ok(())
}

// This function determines whether to start interactive mode or not
// based on if the stdin is a terminal or not
fn start(
    config: &GlobalConfig, // This holds all the configurations
    text: Option<String>,  // This is the prompt
    include: Option<Vec<String>>,
    no_stream: bool, // This boolean tells if the process has a input stream
) -> Result<()> {
    // This checks if the standard input is a terminal
    if stdin().is_terminal() {
        match text {
            // If there is any text, call the start_directive function and passes down all the arguments
            Some(text) => start_directive(config, &text, include, no_stream),
            // If text is none, we call start_interactive function
            None => start_interactive(config),
        }
    } else {
        // If the input is not from the terminal
        let mut input = String::new();
        stdin().read_to_string(&mut input)?;
        if let Some(text) = text {
            // making the input for the LLMs
            input = format!("{text}\n{input}");
        }
        start_directive(config, &input, include, no_stream) // call function which returns a Result
    }
}

// function is responsible for
// processing input data, interacting with a client, and handling output based on certain conditions
fn start_directive(
    config: &GlobalConfig,
    text: &str,
    include: Option<Vec<String>>,
    no_stream: bool,
) -> Result<()> {
    // check if sessing field in config has a value
    if let Some(session) = &config.read().session {
        // If session exists, we call the guard_save method on the session object
        session.guard_save()?;
    }
    // make an input object
    let input = Input::new(text, include.unwrap_or_default())?;
    // make a new client with the given config
    let mut client = init_client(config)?;
    // ensuring that the client has the necessary capabilities to process the input
    ensure_model_capabilities(client.as_mut(), input.required_capabilities())?;
    config.read().maybe_print_send_tokens(&input);
    // This assigns a value to output based on the value of no_stream variable, which is an argument for the function
    let output = if no_stream {
        // if true, send message to client and store the output in variable 'output'
        let output = client.send_message(input.clone())?;
        // check if the output is going to be in termina
        if stdout().is_terminal() {
            // initialize a markdown render object, for printing of the output
            let render_options = config.read().get_render_options()?;
            let mut markdown_render = MarkdownRender::init(render_options)?;
            println!("{}", markdown_render.render(&output).trim());
        } else {
            // else we directly print
            println!("{}", output);
        }
        output // return the output
    } else {
        // if no_stream is false, we create an abort signal
        let abort = create_abort_signal();
        // render the stream of output, using the render_stream function
        render_stream(&input, client.as_ref(), config, abort)?
    };
    // call the save_message method on the config object, passing in the input and the output
    config.write().save_message(input, &output)
}

fn start_interactive(config: &GlobalConfig) -> Result<()> {
    cl100k_base_singleton();
    // Making a Repl - Read Evaluate Print Loop
    let mut repl: Repl = Repl::init(config)?;
    repl.run() // Running the Repl
}
