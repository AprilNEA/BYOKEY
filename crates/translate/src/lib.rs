//! Request and response translators between LLM API formats.
//!
//! This crate provides bidirectional translation between `OpenAI`, Claude, and
//! Gemini message formats. All translators are pure functions with no I/O.

pub mod claude_to_openai;
pub mod gemini_to_openai;
pub mod openai_to_claude;
pub mod openai_to_gemini;
pub mod thinking;

pub use claude_to_openai::ClaudeToOpenAI;
pub use gemini_to_openai::GeminiToOpenAI;
pub use openai_to_claude::OpenAIToClaude;
pub use openai_to_gemini::OpenAIToGemini;
pub use thinking::ThinkingExtractor;
