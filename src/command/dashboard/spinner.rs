//! Spinner animation constants for the dashboard.

/// Braille spinner frames for subtle loading animation
pub const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Number of spinner frames (used by event loop to wrap frame counter)
pub const SPINNER_FRAME_COUNT: u8 = SPINNER_FRAMES.len() as u8;
