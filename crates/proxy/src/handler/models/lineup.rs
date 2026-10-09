//! Claude model order and release dates for `/v1/models`.
//!
//! Model pickers show models in list order. Anthropic lists the newest
//! release first; BYOKEY lists the current lineup first, one model per
//! family in tier order (Fable, Opus, Sonnet, Haiku, as Anthropic's models
//! overview does), then every older model newest release first.

use byokey_types::{ClaudeFamily as Family, ClaudeModel};
use std::cmp::Reverse;
use time::macros::date;

/// Release dates from Anthropic's model pages
/// (<https://platform.claude.com/docs/en/models/overview>).
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

/// The release date of `model`, if known.
#[must_use]
pub(crate) fn released(model: ClaudeModel) -> Option<time::Date> {
    RELEASES
        .iter()
        .find(|(f, v, _)| *f == model.family && *v == model.version)
        .map(|(_, _, d)| *d)
}

/// Where a model goes in the list: the current lineup by tier, older models
/// newest release first, then models without a known release date. Whether
/// a model is its family's current one depends on what else is listed, so
/// keys come from [`Newest::lineup`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Lineup {
    /// The newest listed model of its family.
    Current(Family),
    /// An older model.
    Older {
        released: Reverse<time::Date>,
        version: Reverse<(u8, u8)>,
    },
    /// A model without a known release date.
    Undated,
}

/// The newest version of each listed family.
struct Newest(Vec<(Family, (u8, u8))>);

impl Newest {
    fn of(models: impl IntoIterator<Item = ClaudeModel>) -> Self {
        let mut newest: Vec<(Family, (u8, u8))> = Vec::new();
        for m in models {
            match newest.iter_mut().find(|(f, _)| *f == m.family) {
                Some((_, v)) => *v = (*v).max(m.version),
                None => newest.push((m.family, m.version)),
            }
        }
        Self(newest)
    }

    fn lineup(&self, model: ClaudeModel) -> Lineup {
        if self.0.contains(&(model.family, model.version)) {
            return Lineup::Current(model.family);
        }
        match released(model) {
            Some(d) => Lineup::Older {
                released: Reverse(d),
                version: Reverse(model.version),
            },
            None => Lineup::Undated,
        }
    }
}

/// Sort `models` into lineup order. The sort is stable, so models that
/// compare equal keep their order. Unrecognized model IDs go last.
pub(crate) fn sort<T>(models: &mut [T], model: impl Fn(&T) -> Option<ClaudeModel>) {
    let newest = Newest::of(models.iter().filter_map(&model));
    models.sort_by_cached_key(|m| model(m).map_or(Lineup::Undated, |m| newest.lineup(m)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(ids: &[&str]) -> Vec<String> {
        let mut v: Vec<ClaudeModel> = ids.iter().map(|id| id.parse().unwrap()).collect();
        sort(&mut v, |m| Some(*m));
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn current_lineup_first_then_newest_release_first() {
        assert_eq!(
            sorted(&[
                "claude-fable-5-1",
                "claude-fable-5",
                "claude-opus-4-7",
                "claude-opus-4-8",
                "claude-opus-5-5",
                "claude-opus-5",
                "claude-sonnet-5",
                "claude-haiku-4-5",
            ]),
            [
                "claude-fable-5-1",
                "claude-opus-5-5",
                "claude-sonnet-5",
                "claude-haiku-4-5",
                "claude-opus-5",
                "claude-fable-5",
                "claude-opus-4-8",
                "claude-opus-4-7",
            ]
        );
    }

    #[test]
    fn a_family_is_current_at_its_newest_listed_version() {
        // Without Haiku 4.5 listed, and with Sonnet only at 4.6.
        assert_eq!(
            sorted(&[
                "claude-sonnet-4-5",
                "claude-opus-4-6",
                "claude-sonnet-4-6",
                "claude-opus-5-5",
            ]),
            [
                "claude-opus-5-5",
                "claude-sonnet-4-6",
                "claude-opus-4-6",
                "claude-sonnet-4-5",
            ]
        );
    }

    #[test]
    fn undated_models_go_last_in_their_original_order() {
        assert_eq!(
            sorted(&[
                "claude-opus-4-1",
                "claude-sonnet-3-7",
                "claude-opus-4-7",
                "claude-opus-5-5",
                "claude-sonnet-5",
            ]),
            [
                "claude-opus-5-5",
                "claude-sonnet-5",
                "claude-opus-4-7",
                "claude-opus-4-1",
                "claude-sonnet-3-7",
            ]
        );
    }
}
