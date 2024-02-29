// this file has a code snippet which defines a platform-specific
// function set_text for setting text in the clipboard

// this below line means it will only be compiled if the target OS is neither Android nor Emscripten
#[cfg(not(any(target_os = "android", target_os = "emscripten")))]
// Uses the lazy_static macro to declare a static, lazily-initialized global variable named CLIPBOARD
lazy_static::lazy_static! {
    static ref CLIPBOARD: std::sync::Arc<std::sync::Mutex<Option<arboard::Clipboard>>> =
    std::sync::Arc::new(std::sync::Mutex::new(arboard::Clipboard::new().ok()));
}

// this below line means it will only be compiled if the target OS is neither Android nor Emscripten
#[cfg(not(any(target_os = "android", target_os = "emscripten")))]
// The set_text function is defined with a platform-specific implementation for setting text in the clipboard
pub fn set_text(text: &str) -> anyhow::Result<()> {
    // acquires a lock on the clipboard mutex
    let mut clipboard = CLIPBOARD.lock().unwrap();
    match clipboard.as_mut() {
        // attempting to set the text using the clipboard instance
        Some(clipboard) => clipboard.set_text(text)?,
        // clipboard is unavailable (None), we return an error
        None => anyhow::bail!("No available clipboard"),
    }
    Ok(())
}

// this below line means it will only be compiled if the target OS is neither Android nor Emscripten
#[cfg(any(target_os = "android", target_os = "emscripten"))]
pub fn set_text(_text: &str) -> anyhow::Result<()> {
    // returning an error when no clipboard is available
    anyhow::bail!("No available clipboard")
}
