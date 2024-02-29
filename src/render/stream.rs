use super::{MarkdownRender, ReplyEvent};

use crate::utils::AbortSignal;

use anyhow::Result;
use crossbeam::channel::Receiver;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    queue, style,
    terminal::{self, disable_raw_mode, enable_raw_mode},
};
use std::{
    io::{self, Stdout, Write},
    ops::Div,
    time::{Duration, Instant},
};
use textwrap::core::display_width;

// this function makes the streaming Markdown-rendered text
pub fn markdown_stream(
    rx: &Receiver<ReplyEvent>,
    render: &mut MarkdownRender,
    abort: &AbortSignal,
) -> Result<()> {
    // enabling raw mode for the terminal output
    enable_raw_mode()?;
    let mut stdout = io::stdout();

    // calling the "markdown_stream_inner" for actully making the md text
    let ret = markdown_stream_inner(rx, render, abort, &mut stdout);

    // we disable the raw mode
    disable_raw_mode()?;

    ret
}

// this function streams raw text without any rendering
pub fn raw_stream(rx: &Receiver<ReplyEvent>, abort: &AbortSignal) -> Result<()> {
    // continuously checking for new events from the receiver channel
    loop {
        if abort.aborted() {
            return Ok(());
        }
        if let Ok(evt) = rx.try_recv() {
            match evt {
                // If the event is a text, we print it to stdout
                ReplyEvent::Text(text) => {
                    print!("{}", text);
                }
                // If its a Done event, we breaks the loop
                ReplyEvent::Done => {
                    break;
                }
            }
        }
    }
    Ok(())
}

// this function holds the core logic for streaming Markdown-rendered text
fn markdown_stream_inner(
    rx: &Receiver<ReplyEvent>,
    render: &mut MarkdownRender,
    abort: &AbortSignal,
    writer: &mut Stdout,
) -> Result<()> {
    // initializing variables for tracking time, buffer content, and spinner
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(50);

    let mut buffer = String::new();
    let mut buffer_rows = 1;

    let columns = terminal::size()?.0;

    let mut spinner = Spinner::new(" Generating");

    // initialize a loop to process events
    'outer: loop {
        // if the abort signal is received, return Ok(())
        if abort.aborted() {
            return Ok(());
        }
        //
        spinner.step(writer)?;

        // for all the events that are gathered, we do the following
        for reply_event in gather_events(rx) {
            // we stop the spinner
            spinner.stop(writer)?;

            // processes each text event received
            match reply_event {
                ReplyEvent::Text(text) => {
                    let (col, mut row) = cursor::position()?;

                    // Fix unexpected duplicate lines on kitty, see https://github.com/sigoden/aichat/issues/105
                    if col == 0 && row > 0 && display_width(&buffer) == columns as usize {
                        row -= 1;
                    }

                    // moves the cursor to the appropriate position
                    if row + 1 >= buffer_rows {
                        queue!(writer, cursor::MoveTo(0, row + 1 - buffer_rows),)?;
                    } else {
                        let scroll_rows = buffer_rows - row - 1;
                        queue!(
                            writer,
                            terminal::ScrollUp(scroll_rows),
                            cursor::MoveTo(0, 0),
                        )?;
                    }

                    // No guarantee that text returned by render will not be re-layouted, so it is better to clear it.
                    queue!(writer, terminal::Clear(terminal::ClearType::FromCursorDown))?;

                    // handling cases where the text contains newline characters
                    if text.contains('\n') {
                        let text = format!("{buffer}{text}");
                        let (head, tail) = split_line_tail(&text);
                        let output = render.render(head);
                        print_block(writer, &output, columns)?;
                        buffer = tail.to_string();
                    } else {
                        buffer = format!("{buffer}{text}");
                    }

                    // rendering and then printing the text to stdout
                    let output = render.render_line(&buffer);
                    if output.contains('\n') {
                        let (head, tail) = split_line_tail(&output);
                        buffer_rows = print_block(writer, head, columns)?;
                        queue!(writer, style::Print(&tail),)?;

                        // No guarantee the buffer width of the buffer will not exceed the number of columns.
                        // So we calculate the number of rows needed, rather than setting it directly to 1.
                        buffer_rows += need_rows(tail, columns);
                    } else {
                        queue!(writer, style::Print(&output))?;
                        buffer_rows = need_rows(&output, columns);
                    }

                    writer.flush()?;
                }
                ReplyEvent::Done => {
                    break 'outer;
                }
            }
        }

        // handling keyboard events such as Ctrl+C or Ctrl+D to gracefully terminate the program
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| tick_rate.div(2));
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                        abort.set_ctrlc();
                        break;
                    }
                    KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                        abort.set_ctrld();
                        break;
                    }
                    _ => {}
                }
            }
        }

        // handling timer-based events to refresh the display
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    // once all events are processed
    //  it stops the spinner and
    spinner.stop(writer)?;

    // return Ok(())
    Ok(())
}

// this is struct which represents the spinner
struct Spinner {
    index: usize,
    message: String,
    stopped: bool,
}

impl Spinner {
    const DATA: [&'static str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    // This is the constructor for the struct
    fn new(message: &str) -> Self {
        Spinner {
            index: 0,
            message: message.to_string(),
            stopped: false,
        }
    }

    // This function progresses the spinner animation by one frame
    fn step(&mut self, writer: &mut Stdout) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        // printing the spinner frame, dots, and message to the terminal
        let frame = Self::DATA[self.index % Self::DATA.len()];
        let dots = ".".repeat((self.index / 5) % 4);
        let line = format!("{frame}{}{:<3}", self.message, dots);
        queue!(writer, cursor::MoveToColumn(0), style::Print(line),)?;
        if self.index == 0 {
            queue!(writer, cursor::Hide)?;
        }
        writer.flush()?;
        self.index += 1;
        Ok(())
    }

    // this function stops the spinner animation.
    fn stop(&mut self, writer: &mut Stdout) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        queue!(
            writer,
            cursor::MoveToColumn(0),
            // it clears the spinner from the terminal
            terminal::Clear(terminal::ClearType::FromCursorDown),
            // showing the cursor
            cursor::Show
        )?;
        writer.flush()?;
        // returning
        Ok(())
    }
}

// this function collects all text events from the receiver channel and
// combines them into a single text event and also checks if a "Done" event is received
fn gather_events(rx: &Receiver<ReplyEvent>) -> Vec<ReplyEvent> {
    let mut texts = vec![];
    let mut done = false;
    // iterating over all the events received from the channel
    for reply_event in rx.try_iter() {
        match reply_event {
            // if the event is a text event, we append it to a vector of texts
            ReplyEvent::Text(v) => texts.push(v),
            // If it's a "Done" event, we set a flag
            ReplyEvent::Done => {
                done = true;
            }
        }
    }
    // constructing a vector of events to return,
    let mut events = vec![];
    // combining all texts into a single text event if necessary
    if !texts.is_empty() {
        events.push(ReplyEvent::Text(texts.join("")))
    }
    if done {
        events.push(ReplyEvent::Done)
    }
    // returning
    events
}

// this function prints a block of text to the terminal,
// ensuring that each line is correctly printed even if it exceeds the terminal width
fn print_block(writer: &mut Stdout, text: &str, columns: u16) -> Result<u16> {
    // temperary variable for saving the number of lines printed
    let mut num = 0;
    // iterates over each line in the provided text
    for line in text.split('\n') {
        // printing each line to the terminal
        queue!(
            writer,
            style::Print(line),
            style::Print("\n"),
            cursor::MoveLeft(columns),
        )?;
        // increment the number of lines printed
        num += 1;
    }
    // returns the number of rows printed
    Ok(num)
}

// this function splits a text into its head and tail parts at the last newline character
fn split_line_tail(text: &str) -> (&str, &str) {
    // spliting the text into head and tail parts at the last occurrence of a newline character
    if let Some((head, tail)) = text.rsplit_once('\n') {
        // return head and tail separately
        (head, tail)
    } else {
        // If no newline character is found, we return an empty string for the head
        ("", text)
    }
}

// This function calculates the number of rows needed to display a given text block based on the terminal width
fn need_rows(text: &str, columns: u16) -> u16 {
    let buffer_width = display_width(text).max(1) as u16;
    (buffer_width + columns - 1) / columns
}
