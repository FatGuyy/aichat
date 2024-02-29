mod abort_signal;
mod clipboard;
mod prompt_input;
mod render_prompt;
mod tiktoken;

pub use self::abort_signal::{create_abort_signal, AbortSignal};
pub use self::clipboard::set_text;
pub use self::prompt_input::*;
pub use self::render_prompt::render_prompt;
pub use self::tiktoken::cl100k_base_singleton;

use sha2::{Digest, Sha256};

// this function returns the current local time in RFC 3339 format with seconds precision
pub fn now() -> String {
    let now = chrono::Local::now();
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

// this function constructs an environment variable name by
// combining the crate name and the given key, both converted to uppercase
pub fn get_env_name(key: &str) -> String {
    format!(
        "{}_{}",
        env!("CARGO_CRATE_NAME").to_ascii_uppercase(),
        key.to_ascii_uppercase(),
    )
}

// this function tokenizes the input text using a pre-trained tokenizer i.e. Split text to tokens
pub fn tokenize(text: &str) -> Vec<String> {
    let tokens = cl100k_base_singleton()
        .lock()
        .encode_with_special_tokens(text);
    // Decoding the tokenized bytes back into strings
    let token_bytes: Vec<Vec<u8>> = tokens
        .into_iter()
        .map(|v| cl100k_base_singleton().lock().decode_bytes(vec![v]))
        //  collecting the tokens into a vector
        .collect();
    // declaring the output vector
    let mut output = vec![];
    let mut current_bytes = vec![];
    for bytes in token_bytes {
        // Splits the text into tokens, handling characters properly
        current_bytes.extend(bytes);
        if let Ok(v) = std::str::from_utf8(&current_bytes) {
            // pushing the bytes into output string
            output.push(v.to_string());
            current_bytes.clear();
        }
    }
    // returning the output
    output
}

// this function counts how many tokens a piece of text needs to consume
pub fn count_tokens(text: &str) -> usize {
    cl100k_base_singleton()
        .lock()
        .encode_with_special_tokens(text)
        .len()
}

// this function determines whether a light theme should be used based on the
// background color provided in the colorfgbg string
pub fn light_theme_from_colorfgbg(colorfgbg: &str) -> Option<bool> {
    // Spliting the colorfgbg string by ';' to extract the background color component
    let parts: Vec<_> = colorfgbg.split(';').collect();
    let bg = match parts.len() {
        2 => &parts[1],
        3 => &parts[2],
        _ => {
            return None;
        }
    };
    let bg = bg.parse::<u8>().ok()?;
    // parsing the background color to an ANSI 256 color code and converts it to RGB
    let (r, g, b) = ansi_colours::rgb_from_ansi256(bg);

    // calculating the luminance of the background color
    let v = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    let light = v > 128.0;

    // returns true the v is above certain threshold, indicating light theme
    Some(light)
}

// this function initializes a Tokio runtime with all features enabled for the current thread
pub fn init_tokio_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    use anyhow::Context;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .with_context(|| "Failed to init tokio")
}

// this function computes the SHA-256 hash of the input string and returns it as a hexadecimal string
pub fn sha256sum(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let result = hasher.finalize();
    format!("{:x}", result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        assert_eq!(tokenize("😊 hello world"), ["😊", " hello", " world"]);
        assert_eq!(tokenize("世界"), ["世", "界"]);
    }

    #[test]
    fn test_count_tokens() {
        assert_eq!(count_tokens("😊 hello world"), 4);
    }
}
