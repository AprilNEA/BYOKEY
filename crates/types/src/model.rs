//! Claude models by family and version, whatever the spelling.
//!
//! Anthropic names a model `claude-opus-5-5`; Copilot spells it
//! `claude-opus-5.5`, and dated snapshots add `-20251001`. [`ClaudeModel`]
//! reads every spelling and writes Anthropic's, which is the id clients such
//! as Claude Desktop recognise.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// A Claude model family, highest tier first. Written as Claude Code's
/// family aliases are: `fable`, `opus`, `sonnet`, `haiku`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClaudeFamily {
    Fable,
    Opus,
    Sonnet,
    Haiku,
}

/// A name that is not a [`ClaudeFamily`].
#[derive(Debug, thiserror::Error)]
#[error("unknown model family `{0}`; expected fable, opus, sonnet or haiku")]
pub struct UnknownFamily(String);

impl ClaudeFamily {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "fable" => Self::Fable,
            "opus" => Self::Opus,
            "sonnet" => Self::Sonnet,
            "haiku" => Self::Haiku,
            _ => return None,
        })
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fable => "fable",
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
        }
    }

    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Fable => "Fable",
            Self::Opus => "Opus",
            Self::Sonnet => "Sonnet",
            Self::Haiku => "Haiku",
        }
    }
}

impl fmt::Display for ClaudeFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ClaudeFamily {
    type Err = UnknownFamily;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| UnknownFamily(s.to_owned()))
    }
}

impl Serialize for ClaudeFamily {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ClaudeFamily {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A Claude model: family and version, without snapshot date or provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClaudeModel {
    pub family: ClaudeFamily,
    pub version: (u8, u8),
}

/// A model id as far as [`ClaudeModel::parse_id`] understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedId {
    pub model: ClaudeModel,
    /// A suffix beyond version and snapshot date, such as Copilot's `-fast`:
    /// a different model than [`ParsedId::model`] names.
    pub variant: bool,
}

impl ClaudeModel {
    /// Parse `claude-<family>-<major>[-.]<minor>[-<snapshot>][-<variant>]`.
    #[must_use]
    pub fn parse_id(id: &str) -> Option<ParsedId> {
        let id = id.strip_prefix("claude-")?;
        let (family, rest) = id.split_once('-')?;
        let family = ClaudeFamily::parse(family)?;
        let mut parts = rest.split(['-', '.']).peekable();
        let major = parts.next()?.parse().ok()?;
        let minor = parts
            .next_if(|p| p.len() <= 2 && p.parse::<u8>().is_ok())
            .map_or(0, |p| p.parse().unwrap_or(0));
        // An 8-digit snapshot date (`-20251101`) names the same model.
        parts.next_if(|p| p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit()));
        Some(ParsedId {
            model: Self {
                family,
                version: (major, minor),
            },
            variant: parts.next().is_some(),
        })
    }

    /// The model `id` names exactly: any spelling of it, but not a variant.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::parse_id(id).filter(|p| !p.variant).map(|p| p.model)
    }

    /// The name Anthropic gives the model: `Claude Opus 5.5`, `Claude Sonnet 5`.
    #[must_use]
    pub fn display_name(&self) -> String {
        let family = self.family.display_name();
        match self.version {
            (major, 0) => format!("Claude {family} {major}"),
            (major, minor) => format!("Claude {family} {major}.{minor}"),
        }
    }
}

impl fmt::Display for ClaudeModel {
    /// Anthropic's id: `claude-opus-5-5`, `claude-sonnet-5`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (major, minor) = self.version;
        write!(f, "claude-{}-{major}", self.family.as_str())?;
        if minor != 0 {
            write!(f, "-{minor}")?;
        }
        Ok(())
    }
}

impl FromStr for ClaudeModel {
    type Err = crate::ByokError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_id(s).ok_or_else(|| crate::ByokError::UnsupportedModel(s.to_owned()))
    }
}

impl Serialize for ClaudeModel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ClaudeModel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus(major: u8, minor: u8) -> ClaudeModel {
        ClaudeModel {
            family: ClaudeFamily::Opus,
            version: (major, minor),
        }
    }

    #[test]
    fn parses_every_spelling() {
        let parsed = |id| ClaudeModel::parse_id(id).map(|p| (p.model, p.variant));
        assert_eq!(parsed("claude-opus-5-5"), Some((opus(5, 5), false)));
        assert_eq!(parsed("claude-opus-5.5"), Some((opus(5, 5), false)));
        assert_eq!(parsed("claude-opus-5"), Some((opus(5, 0), false)));
        assert_eq!(
            parsed("claude-opus-4-5-20251101"),
            Some((opus(4, 5), false))
        );
        assert_eq!(parsed("claude-opus-4.8-fast"), Some((opus(4, 8), true)));
        assert_eq!(parsed("claude-opus-5-5-low-fast"), Some((opus(5, 5), true)));
        assert_eq!(
            ClaudeModel::parse_id("claude-sonnet-4").map(|p| p.model.version),
            Some((4, 0))
        );
        assert_eq!(ClaudeModel::parse_id("gpt-5.4"), None);
        assert_eq!(ClaudeModel::parse_id("claude-instant-1"), None);
        assert_eq!(ClaudeModel::parse_id("cursor/claude-opus-5-5"), None);
    }

    #[test]
    fn writes_anthropic_ids() {
        for (id, canonical) in [
            ("claude-opus-5.5", "claude-opus-5-5"),
            ("claude-sonnet-5", "claude-sonnet-5"),
            ("claude-haiku-4-5-20251001", "claude-haiku-4-5"),
            ("claude-fable-5.1", "claude-fable-5-1"),
        ] {
            assert_eq!(ClaudeModel::from_id(id).unwrap().to_string(), canonical);
        }
        assert_eq!(
            ClaudeModel::from_id("claude-opus-4.8-fast"),
            None,
            "a variant is not its base model"
        );
        assert_eq!(opus(5, 5).display_name(), "Claude Opus 5.5");
        assert_eq!(opus(5, 0).display_name(), "Claude Opus 5");
    }

    #[test]
    fn families_are_named_by_their_alias() {
        let f: ClaudeFamily = serde_json::from_str(r#""opus""#).unwrap();
        assert_eq!(f, ClaudeFamily::Opus);
        assert_eq!(serde_json::to_string(&f).unwrap(), r#""opus""#);
        let err = "claude-opus".parse::<ClaudeFamily>().unwrap_err();
        assert!(err.to_string().contains("expected fable, opus"), "{err}");
    }

    #[test]
    fn serde_takes_any_spelling_and_writes_anthropic_ids() {
        let m: ClaudeModel = serde_json::from_str(r#""claude-opus-5.5""#).unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), r#""claude-opus-5-5""#);
        assert!(serde_json::from_str::<ClaudeModel>(r#""gpt-5.4""#).is_err());
    }
}
