//! Claude model order and release dates for `/v1/models`.
//!
//! Model pickers show models in list order. Anthropic lists the newest
//! release first; BYOKEY lists the current lineup first, one model per
//! family in tier order (Fable, Opus, Sonnet, Haiku, as Anthropic's models
//! overview does), then every older model newest release first. Copilot and
//! Cursor spell the same models with dots or dashes, so ids are compared by
//! family and version, not by string.

use std::cmp::Reverse;
use time::macros::date;

/// A Claude model family, highest tier first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Fable,
    Opus,
    Sonnet,
    Haiku,
}

impl Family {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "fable" => Self::Fable,
            "opus" => Self::Opus,
            "sonnet" => Self::Sonnet,
            "haiku" => Self::Haiku,
            _ => return None,
        })
    }
}

/// A Claude model as named in an id: family, version, and whether it is a
/// variant such as Copilot's `-fast`.
#[derive(Debug, PartialEq, Eq)]
struct Model {
    family: Family,
    version: (u8, u8),
    variant: bool,
}

impl Model {
    /// Parse `claude-<family>-<major>[-.]<minor>[-<snapshot>][-<variant>]`,
    /// with or without a `provider/` prefix.
    fn parse(id: &str) -> Option<Self> {
        let id = id.rsplit('/').next()?.strip_prefix("claude-")?;
        let (family, rest) = id.split_once('-')?;
        let family = Family::parse(family)?;
        let mut parts = rest.split(['-', '.']).peekable();
        let major = parts.next()?.parse().ok()?;
        let minor = parts
            .next_if(|p| p.len() <= 2 && p.parse::<u8>().is_ok())
            .map_or(0, |p| p.parse().unwrap_or(0));
        // An 8-digit snapshot date (`-20251101`) names the same model.
        parts.next_if(|p| p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit()));
        Some(Self {
            family,
            version: (major, minor),
            variant: parts.next().is_some(),
        })
    }
}

/// Release dates from Anthropic's model pages
/// (<https://platform.claude.com/docs/en/models/overview>). A model missing
/// here is listed after the dated ones.
const RELEASES: &[(Family, (u8, u8), time::Date)] = &[
    (Family::Opus, (5, 5), date!(2026 - 09 - 22)),
    (Family::Fable, (5, 1), date!(2026 - 09 - 01)),
    (Family::Opus, (5, 0), date!(2026 - 07 - 24)),
    (Family::Sonnet, (5, 0), date!(2026 - 06 - 30)),
    (Family::Fable, (5, 0), date!(2026 - 06 - 09)),
    (Family::Opus, (4, 8), date!(2026 - 05 - 28)),
    (Family::Opus, (4, 7), date!(2026 - 04 - 16)),
    (Family::Sonnet, (4, 6), date!(2026 - 02 - 17)),
    (Family::Opus, (4, 6), date!(2026 - 02 - 05)),
    (Family::Opus, (4, 5), date!(2025 - 11 - 24)),
    (Family::Haiku, (4, 5), date!(2025 - 10 - 15)),
    (Family::Sonnet, (4, 5), date!(2025 - 09 - 29)),
    (Family::Opus, (4, 0), date!(2025 - 05 - 22)),
    (Family::Sonnet, (4, 0), date!(2025 - 05 - 22)),
];

fn released(family: Family, version: (u8, u8)) -> Option<time::Date> {
    RELEASES
        .iter()
        .find(|(f, v, _)| *f == family && *v == version)
        .map(|(_, _, d)| *d)
}

/// The release date of the model `id` names, if known.
#[must_use]
pub(super) fn released_on(id: &str) -> Option<time::Date> {
    let model = Model::parse(id)?;
    released(model.family, model.version)
}

/// Sort one provider's models into lineup order: the newest model of each
/// family in tier order, then the rest newest release first, each variant
/// right after its base model. Undated Claude models follow, then non-Claude
/// ids, each in their original order.
pub(super) fn sort<T>(models: &mut [T], id: impl Fn(&T) -> &str) {
    let mut newest: Vec<(Family, (u8, u8))> = Vec::new();
    for m in models.iter().filter_map(|m| Model::parse(id(m))) {
        match newest.iter_mut().find(|(f, _)| *f == m.family) {
            Some((_, v)) => *v = (*v).max(m.version),
            None => newest.push((m.family, m.version)),
        }
    }
    models.sort_by_cached_key(|m| {
        let Some(m) = Model::parse(id(m)) else {
            return (3, None, None, Reverse((0, 0)), false);
        };
        let current = newest.contains(&(m.family, m.version));
        let date = released(m.family, m.version);
        let group = if current {
            0
        } else if date.is_some() {
            1
        } else {
            2
        };
        (
            group,
            current.then_some(m.family),
            date.map(Reverse),
            Reverse(m.version),
            m.variant,
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(ids: &[&'static str]) -> Vec<&'static str> {
        let mut v = ids.to_vec();
        sort(&mut v, |s| s);
        v
    }

    #[test]
    fn parses_every_spelling() {
        let opus = |major, minor, variant| {
            Some(Model {
                family: Family::Opus,
                version: (major, minor),
                variant,
            })
        };
        assert_eq!(Model::parse("claude-opus-5-5"), opus(5, 5, false));
        assert_eq!(Model::parse("cursor/claude-opus-5.5"), opus(5, 5, false));
        assert_eq!(Model::parse("claude-opus-5"), opus(5, 0, false));
        assert_eq!(Model::parse("claude-opus-4-5-20251101"), opus(4, 5, false));
        assert_eq!(Model::parse("claude-opus-4.8-fast"), opus(4, 8, true));
        assert_eq!(Model::parse("claude-opus-5-5-low-fast"), opus(5, 5, true));
        assert_eq!(
            Model::parse("claude-sonnet-4").map(|m| m.version),
            Some((4, 0))
        );
        assert_eq!(Model::parse("gpt-5.4"), None);
        assert_eq!(Model::parse("claude-instant-1"), None);
    }

    #[test]
    fn current_lineup_first_then_newest_release_first() {
        // Copilot's catalog order.
        let copilot = [
            "claude-fable-5.1",
            "claude-fable-5",
            "claude-opus-4.7",
            "claude-opus-4.8-fast",
            "claude-opus-4.8",
            "claude-opus-5.5",
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-haiku-4.5",
        ];
        assert_eq!(
            sorted(&copilot),
            [
                "claude-fable-5.1",
                "claude-opus-5.5",
                "claude-sonnet-5",
                "claude-haiku-4.5",
                "claude-opus-5",
                "claude-fable-5",
                "claude-opus-4.8",
                "claude-opus-4.8-fast",
                "claude-opus-4.7",
            ]
        );
    }

    #[test]
    fn a_family_is_current_at_its_newest_listed_version() {
        // Without Haiku 4.5 listed, and with Sonnet only at 4.6.
        assert_eq!(
            sorted(&[
                "cursor/claude-sonnet-4-5",
                "cursor/claude-opus-4-6",
                "cursor/claude-sonnet-4-6",
                "cursor/claude-opus-5-5",
            ]),
            [
                "cursor/claude-opus-5-5",
                "cursor/claude-sonnet-4-6",
                "cursor/claude-opus-4-6",
                "cursor/claude-sonnet-4-5",
            ]
        );
    }

    #[test]
    fn undated_and_unknown_models_go_last_in_their_original_order() {
        assert_eq!(
            sorted(&[
                "auto-smart",
                "claude-opus-9-9",
                "claude-opus-4-7",
                "claude-opus-4-1",
                "gpt-5.4",
            ]),
            [
                "claude-opus-9-9",
                "claude-opus-4-7",
                "claude-opus-4-1",
                "auto-smart",
                "gpt-5.4",
            ]
        );
    }

    #[test]
    fn release_dates_match_any_spelling() {
        assert_eq!(released_on("claude-opus-5-5"), Some(date!(2026 - 09 - 22)));
        assert_eq!(
            released_on("cursor/claude-opus-5.5"),
            released_on("claude-opus-5-5")
        );
        assert_eq!(released_on("claude-opus-4-1"), None);
        assert_eq!(released_on("composer-2.5"), None);
    }
}
