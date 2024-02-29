mod markdown;
mod stream;

pub use self::markdown::{MarkdownRender, RenderOptions};
use self::stream::{markdown_stream, raw_stream};

use crate::client::Client;
use crate::config::{GlobalConfig, Input};
use crate::utils::AbortSignal;

use anyhow::{Context, Result};
use crossbeam::channel::{unbounded, Sender};
use crossbeam::sync::WaitGroup;
use is_terminal::IsTerminal;
use nu_ansi_term::{Color, Style};
use std::io::stdout;
use std::thread::spawn;

// this function renders a stream of messages based on the input
pub fn render_stream(
    input: &Input,
    client: &dyn Client,
    config: &GlobalConfig,
    abort: AbortSignal,
) -> Result<String> {
    // creating a wait group wg to synchronize the rendering process
    let wg = WaitGroup::new();
    let wg_cloned = wg.clone();
    let render_options = config.read().get_render_options()?;
    let mut stream_handler = {
        let (tx, rx) = unbounded();
        let abort_clone = abort.clone();
        let highlight = config.read().highlight;
        // spawning a new thread to handle the rendering process
        spawn(move || {
            // Depending on whether the standard output is a terminal or not,
            // we initialize either a Markdown renderer or a raw stream renderer
            let run = move || {
                if stdout().is_terminal() {
                    let mut render = MarkdownRender::init(render_options)?;
                    markdown_stream(&rx, &mut render, &abort)
                } else {
                    // the raw stream renderer
                    raw_stream(&rx, &abort)
                }
            };
            if let Err(err) = run() {
                render_error(err, highlight);
            }
            drop(wg_cloned);
        });
        ReplyHandler::new(tx, abort_clone)
    };
    // sending the input message stream to the client, passing a reply handler to process the stream
    let ret = client.send_message_streaming(input, &mut stream_handler);
    wg.wait();
    // After waiting for the rendering process to finish, we return the rendered output or an error
    let output = stream_handler.get_buffer().to_string();
    match ret {
        Ok(_) => {
            // if no error, we return the renderer
            println!();
            Ok(output)
        }
        Err(err) => {
            // if we have an error, we return the error
            if !output.is_empty() {
                println!();
            }
            Err(err)
        }
    }
}

// This function handles rendering errors
pub fn render_error(err: anyhow::Error, highlight: bool) {
    // formating the error message and prints it to standard error output
    let err = format!("{err:?}");
    if highlight {
        // if highlighting is enabled, we format the error message with a red color
        let style = Style::new().fg(Color::Red);
        eprintln!("{}", style.paint(err));
    } else {
        eprintln!("{err}");
    }
}

// This struct handles the reply events received during rendering
pub struct ReplyHandler {
    sender: Sender<ReplyEvent>,
    buffer: String,
    abort: AbortSignal,
}

impl ReplyHandler {
    // this function initializes a new "ReplyHandler" with a sender and an abort signal
    pub fn new(sender: Sender<ReplyEvent>, abort: AbortSignal) -> Self {
        Self {
            sender,
            abort,
            buffer: String::new(),
        }
    }

    // this function sends text reply events to the sender and appends the text to the buffer
    pub fn text(&mut self, text: &str) -> Result<()> {
        debug!("ReplyText: {}", text);
        if text.is_empty() {
            return Ok(());
        }
        self.buffer.push_str(text);
        let ret = self
            .sender
            .send(ReplyEvent::Text(text.to_string()))
            .with_context(|| "Failed to send ReplyEvent:Text");
        self.safe_ret(ret)?;
        Ok(())
    }

    // this functon sends a done event to the sender
    pub fn done(&mut self) -> Result<()> {
        debug!("ReplyDone");
        let ret = self
            .sender
            .send(ReplyEvent::Done)
            .with_context(|| "Failed to send ReplyEvent::Done");
        self.safe_ret(ret)?;
        Ok(())
    }

    // this function returns a reference to the buffer
    pub fn get_buffer(&self) -> &str {
        &self.buffer
    }

    // this function returns a clone of the abort signal
    pub fn get_abort(&self) -> AbortSignal {
        self.abort.clone()
    }

    // this function handles the result of sending events,
    // ensuring that it returns Ok if the sending is successful and the abort signal is not triggered
    fn safe_ret(&self, ret: Result<()>) -> Result<()> {
        if ret.is_err() && self.abort.aborted() {
            return Ok(());
        }
        ret
    }
}

// This enum represents different types of reply events, including text and done events
pub enum ReplyEvent {
    Text(String),
    Done,
}
