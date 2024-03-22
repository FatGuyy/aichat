use anyhow::{anyhow, Context, Result};
use crossterm::style::{Color, Stylize};
use crossterm::terminal;
use lazy_static::lazy_static;
use std::collections::HashMap;
use syntect::highlighting::{Color as SyntectColor, FontStyle, Style, Theme};
use syntect::parsing::SyntaxSet;
use syntect::{easy::HighlightLines, parsing::SyntaxReference};

/// Comes from https://github.com/sharkdp/bat/raw/5e77ca37e89c873e4490b42ff556370dc5c6ba4f/assets/syntaxes.bin
// This contains the binary data of syntaxes used for syntax highlighting.
// It's loaded from an external file using include_bytes!
const SYNTAXES: &[u8] = include_bytes!("../../assets/syntaxes.bin");

// This macro is used to create a lazily initialized static variable
lazy_static! {
    static ref LANG_MAPS: HashMap<String, String> = {
        let mut m = HashMap::new();
        m.insert("csharp".into(), "C#".into());
        m.insert("php".into(), "PHP Source".into());
        m
    };
}

// this struct is reponsible for rendering in the Markdown format
pub struct MarkdownRender {
    options: RenderOptions,
    syntax_set: SyntaxSet,
    code_color: Option<Color>,
    md_syntax: SyntaxReference,
    code_syntax: Option<SyntaxReference>,
    prev_line_type: LineType,
    wrap_width: Option<u16>,
}

impl MarkdownRender {
    // this funciton deserializes the syntaxes from the binary dat
    pub fn init(options: RenderOptions) -> Result<Self> {
        let syntax_set: SyntaxSet = bincode::deserialize_from(SYNTAXES)
            .with_context(|| "MarkdownRender: invalid syntaxes binary")?;

        // setting the code color from options
        let code_color = options.theme.as_ref().map(get_code_color);
        // getting the Markdown syntax
        let md_syntax = syntax_set.find_syntax_by_extension("md").unwrap().clone();
        let line_type = LineType::Normal;
        // determining the wrap_width based on the wrap field of the options
        let wrap_width = match options.wrap.as_deref() {
            None => None,
            Some(value) => match terminal::size() {
                Ok((columns, _)) => {
                    if value == "auto" {
                        Some(columns)
                    } else {
                        let value = value
                            .parse::<u16>()
                            .map_err(|_| anyhow!("Invalid wrap value"))?;
                        Some(columns.min(value))
                    }
                }
                Err(_) => None,
            },
        };
        // returning Self, wrapped in a result
        Ok(Self {
            syntax_set,
            code_color,
            md_syntax,
            code_syntax: None,
            prev_line_type: line_type,
            wrap_width,
            options,
        })
    }

    // this function splits the input text into lines and put them in the render_line_mut function
    pub fn render(&mut self, text: &str) -> String {
        text.split('\n')
            .map(|line| self.render_line_mut(line))
            .collect::<Vec<String>>()
            .join("\n")
    }

    // this function is for analyzing the line and updating internal state variables based on its type and content
    pub fn render_line(&self, line: &str) -> String {
        let (_, code_syntax, is_code) = self.check_line(line); // check_line function determines whether line contains code or not
        if is_code {
            // if its code, we highlight as per code
            self.highlight_code_line(line, &code_syntax)
        } else {
            self.highlight_line(line, &self.md_syntax, false)
        }
    }

    // this function determines whether the line contains code or not
    fn render_line_mut(&mut self, line: &str) -> String {
        let (line_type, code_syntax, is_code) = self.check_line(line);
        let output = if is_code {
            // if its code, we highlight as per code
            self.highlight_code_line(line, &code_syntax)
        } else {
            self.highlight_line(line, &self.md_syntax, false)
        };
        self.prev_line_type = line_type;
        self.code_syntax = code_syntax;
        output
    }

    // this analyzes a line of text to determine its type and whether it contains code
    fn check_line(&self, line: &str) -> (LineType, Option<SyntaxReference>, bool) {
        // holding previous type of the line
        let mut line_type = self.prev_line_type;
        let mut code_syntax = self.code_syntax.clone();
        // variable indicating if the line is code, for returning
        let mut is_code = false;
        // checking if the line is code using detect_code_block
        if let Some(lang) = detect_code_block(line) {
            match line_type {
                // Normal/does not contain a code block
                LineType::Normal | LineType::CodeEnd => {
                    // updating the line type
                    line_type = LineType::CodeBegin;
                    // checking if the code syntax is not yet determined
                    code_syntax = if lang.is_empty() {
                        None
                    } else {
                        self.find_syntax(&lang).cloned()
                    };
                }
                // line is code
                LineType::CodeBegin | LineType::CodeInner => {
                    // updating the line type
                    line_type = LineType::CodeEnd;
                    code_syntax = None;
                }
            }
        } else {
            // if we don't find a match by detect_code_block, we use the previous line_type to generate output
            match line_type {
                // if it's normal, we do nothing
                LineType::Normal => {}
                // if it's end of code, we update the line_type to Normal
                LineType::CodeEnd => {
                    line_type = LineType::Normal;
                }
                // if it's the beginning of the code
                LineType::CodeBegin => {
                    // syntax is none
                    if code_syntax.is_none() {
                        // we try to find syntax, using find_syntax_by_first_line
                        if let Some(syntax) = self.syntax_set.find_syntax_by_first_line(line) {
                            // if we find this syntax, we update the code_syntax
                            code_syntax = Some(syntax.clone());
                        }
                    }
                    // update the line_type
                    line_type = LineType::CodeInner;
                    // making the is_code true
                    is_code = true;
                }
                // if the line is inner code, we make the is_code true
                LineType::CodeInner => {
                    is_code = true;
                }
            }
        }
        // returnin the tuple
        (line_type, code_syntax, is_code)
    }

    // This applies syntax highlighting to a line of text
    fn highlight_line(&self, line: &str, syntax: &SyntaxReference, is_code: bool) -> String {
        // extracting whitespaces at the beginning of the line
        let ws: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        // trimming the leading whitespace from the line
        let trimmed_line: &str = &line[ws.len()..];
        let mut line_highlighted = None;
        // Checking if a theme is specified
        if let Some(theme) = &self.options.theme {
            // creating a HighlightLines instance with the syntax and theme
            let mut highlighter = HighlightLines::new(syntax, theme);
            // highlighting the trimmed line
            if let Ok(ranges) = highlighter.highlight_line(trimmed_line, &self.syntax_set) {
                line_highlighted = Some(format!("{ws}{}", as_terminal_escaped(&ranges)))
            }
        }
        // if no highlighting is applied, we use the original line
        let line = line_highlighted.unwrap_or_else(|| line.into());
        self.wrap_line(line, is_code)
    }

    fn highlight_code_line(&self, line: &str, code_syntax: &Option<SyntaxReference>) -> String {
        // if syntax reference is available, we highlight the line as code
        if let Some(syntax) = code_syntax {
            self.highlight_line(line, syntax, true)
        }
        // else
        else {
            // applying code color if specified
            let line = match self.code_color {
                Some(color) => line.with(color).to_string(),
                None => line.to_string(),
            };
            self.wrap_line(line, true)
        }
    }

    fn wrap_line(&self, line: String, is_code: bool) -> String {
        // if wrap width is set to 'width'
        if let Some(width) = self.wrap_width {
            // if the line is code and is not wrapped
            if is_code && !self.options.wrap_code {
                // we return the line unchanged
                return line;
            }
            // wrapping the line with the above width
            wrap(&line, width as usize)
        } else {
            // returning the line unchanged
            line
        }
    }

    fn find_syntax(&self, lang: &str) -> Option<&SyntaxReference> {
        // Checking if a language mapping is available for the given language
        if let Some(new_lang) = LANG_MAPS.get(&lang.to_ascii_lowercase()) {
            // finding the syntax reference
            self.syntax_set.find_syntax_by_name(new_lang)
        } else {
            // attempting to find syntax by token or extension
            self.syntax_set
                .find_syntax_by_token(lang)
                .or_else(|| self.syntax_set.find_syntax_by_extension(lang))
        }
    }
}

// this function wraps text to fit within a specified width
fn wrap(text: &str, width: usize) -> String {
    // calculating the indentation of the text by counting leading whitespace characters
    let indent: usize = text.chars().take_while(|c| *c == ' ').count();
    let wrap_options = textwrap::Options::new(width)
        .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit)
        .initial_indent(&text[0..indent]);
    // wrapping the text using the specified options and joining the resulting lines with new characters
    textwrap::wrap(&text[indent..], wrap_options).join("\n")
}

// This struct represents rendering options for the highlighting
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub theme: Option<Theme>,
    pub wrap: Option<String>,
    pub wrap_code: bool,
}

impl RenderOptions {
    // constructor for the RenderOptions struct
    pub(crate) fn new(theme: Option<Theme>, wrap: Option<String>, wrap_code: bool) -> Self {
        Self {
            theme,
            wrap,
            wrap_code,
        }
    }
}

// enum for different types of lines that can occur during rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineType {
    Normal,
    CodeBegin,
    CodeInner,
    CodeEnd,
}

// function for converting highlighted text with styles into terminal-escaped text
fn as_terminal_escaped(ranges: &[(Style, &str)]) -> String {
    let mut output = String::new();
    for (style, text) in ranges {
        let fg = blend_fg_color(style.foreground, style.background);
        let mut text = text.with(convert_color(fg));
        if style.font_style.contains(FontStyle::BOLD) {
            text = text.bold();
        }
        if style.font_style.contains(FontStyle::UNDERLINE) {
            text = text.underlined();
        }
        output.push_str(&text.to_string());
    }
    output
}

// This function converts a `SyntectColor` (color representation of syntect) into a Color
const fn convert_color(c: SyntectColor) -> Color {
    // SyntectColor, has red (r), green (g), and blue (b) components
    // we simply make a Color using these components
    Color::Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

// I used gpt for most of this function coz i really don't get it very well
// This function blends two colors, the foreground color (fg) and the background color (bg),
// how? : based on the alpha value of the foreground color
fn blend_fg_color(fg: SyntectColor, bg: SyntectColor) -> SyntectColor {
    // If the alpha value of the foreground color (fg.a) is fully opaque (255 or 0xff),
    // we returns the foreground color as is
    if fg.a == 0xff {
        return fg;
    }
    // if its not opaque,
    // It then linearly interpolates between the foreground and background colors' RGB components
    // based on this ratio to obtain a blended color

    let ratio = u32::from(fg.a);
    let r = (u32::from(fg.r) * ratio + u32::from(bg.r) * (255 - ratio)) / 255;
    let g = (u32::from(fg.g) * ratio + u32::from(bg.g) * (255 - ratio)) / 255;
    let b = (u32::from(fg.b) * ratio + u32::from(bg.b) * (255 - ratio)) / 255;
    // In maths, linear interpolation is a method of curve fitting using linear polynomials to construct new data points
    // within the range of a discrete set of known data points
    // In color context, linear interpolation takes every value between the minimum and maximum values and assigns
    // it a color between the brightest and darkest colors in a linear way
    SyntectColor {
        r: u8::try_from(r).unwrap_or(u8::MAX),
        g: u8::try_from(g).unwrap_or(u8::MAX),
        b: u8::try_from(b).unwrap_or(u8::MAX),
        a: 255,
    }
    // then we simply just make a SyntectColor instance and return it
}

// This function detects code blocks in a line of text
fn detect_code_block(line: &str) -> Option<String> {
    // checking if line starts with ```
    if !line.starts_with("```") {
        // if not, we return None
        return None;
    }
    // extracting the language identifier
    let lang = line
        .chars()
        .skip(3) // skipping 3 chars (```)
        .take_while(|v| v.is_alphanumeric())
        .collect();
    // collect and return
    Some(lang)
}

// this function retrieves the color used for displaying code blocks in a given theme
fn get_code_color(theme: &Theme) -> Color {
    // searching for a style in theme's scope
    // by the selector of "string"
    let scope = theme.scopes.iter().find(|v| {
        v.scope
            .selectors
            .iter()
            .any(|v| v.path.scopes.iter().any(|v| v.to_string() == "string"))
    });
    // if we find a style
    scope
        // we retrieve foreground color
        .and_then(|v| v.style.foreground)
        // else, we just map it to yellow
        .map_or_else(|| Color::Yellow, convert_color)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"
To unzip a file in Rust, you can use the `zip` crate. Here's an example code that shows how to unzip a file:

```rust
use std::fs::File;

fn unzip_file(path: &str, output_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}
```
"#;
    const TEXT_NO_WRAP_CODE: &str = r#"
To unzip a file in Rust, you can use the `zip` crate. Here's an example code
that shows how to unzip a file:

```rust
use std::fs::File;

fn unzip_file(path: &str, output_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}
```
"#;

    const TEXT_WRAP_ALL: &str = r#"
To unzip a file in Rust, you can use the `zip` crate. Here's an example code
that shows how to unzip a file:

```rust
use std::fs::File;

fn unzip_file(path: &str, output_dir: &str) -> Result<(), Box<dyn
std::error::Error>> {
    todo!()
}
```
"#;

    #[test]
    fn test_render() {
        let options = RenderOptions::default();
        let render = MarkdownRender::init(options).unwrap();
        assert!(render.find_syntax("csharp").is_some());
    }

    #[test]
    fn no_theme() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render(TEXT);
        assert_eq!(TEXT, output);
    }

    #[test]
    fn no_wrap_code() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        render.wrap_width = Some(80);
        let output = render.render(TEXT);
        assert_eq!(TEXT_NO_WRAP_CODE, output);
    }

    #[test]
    fn wrap_all() {
        let options = RenderOptions {
            wrap_code: true,
            ..Default::default()
        };
        let mut render = MarkdownRender::init(options).unwrap();
        render.wrap_width = Some(80);
        let output = render.render(TEXT);
        assert_eq!(TEXT_WRAP_ALL, output);
    }
}
