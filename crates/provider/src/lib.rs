//! Upstream access for BYOKEY's providers.
//!
//! ## Module layout
//!
//! - [`claude`]         — Anthropic API headers: version, betas, Claude Code fingerprint.
//! - [`cloak`]          — Claude Code billing header and tool-name remapping for OAuth.
//! - [`copilot`]        — GitHub Copilot credentials, client identity and model catalog.
//! - [`cursor`]         — Cursor agent API client.
//! - [`device_profile`] — Stable per-scope device fingerprints for Claude headers.
//! - [`registry`]       — Anthropic model ids and their thinking capabilities.

pub mod claude;
pub mod cloak;
pub mod copilot;
pub mod cursor;
pub mod device_profile;
pub mod registry;

pub use copilot::{
    Conversation, CopilotCredentials, CopilotDevice, CopilotIdentity, CopilotModel,
    CopilotUpstream, CopilotVersions,
};
pub use cursor::CursorUpstream;
pub use device_profile::DeviceProfileCache;
pub use registry::{ModelEntry, all_models, parse_qualified_model, thinking_capability};
