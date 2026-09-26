//! Cursor's model catalog (`aiserver.v1.AiService/AvailableModels`) and model
//! name resolution.
//!
//! Cursor names a model plus parameters (`thinking`, `effort`, `fast`, …); the
//! catalog lists each model's parameters and named variants such as
//! `claude-opus-5-5-high-fast`. Any variant name, alias, or
//! `<model>-<value>-<value>` spelling resolves to a model id and parameters.

use super::pb::{Fields, Msg};
use byokey_types::{ByokError, Result};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_mins(15);

/// Model parameters (`effort`, `thinking`, `fast`, …) in the order Cursor
/// lists them, each key at most once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(Vec<(String, String)>);

impl Params {
    /// The value of `key`, if set.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Set `key` to `value`, in place if it is already set.
    pub fn set(&mut self, key: &str, value: &str) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => value.clone_into(&mut slot.1),
            None => self.0.push((key.to_owned(), value.to_owned())),
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `(key, value)` pairs in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Params {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(pairs: I) -> Self {
        let mut params = Self::default();
        for (k, v) in pairs {
            params.set(&k.into(), &v.into());
        }
        params
    }
}

/// A parameter a model takes and the values it allows, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParamOption {
    id: String,
    values: Vec<String>,
}

/// A name that selects a model with given parameters: a variant, alias or
/// legacy slug.
#[derive(Debug, Clone)]
struct Spelling {
    name: String,
    params: Params,
}

/// One model and every spelling that selects it.
#[derive(Debug, Clone)]
struct Model {
    id: String,
    /// Human-readable name, e.g. `Claude Opus 5.5`.
    display: String,
    options: Vec<ParamOption>,
    /// Parameters sent when the caller names only the model.
    defaults: Params,
    names: Vec<Spelling>,
}

impl Model {
    /// The parameter that takes `value`, if any.
    fn option_taking(&self, value: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.values.iter().any(|v| v == value))
            .map(|o| o.id.as_str())
    }
}

/// A resolved model: what goes on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub params: Params,
}

/// The catalog and when it was fetched.
type Cached = Option<(Instant, Vec<Model>)>;

static CACHE: LazyLock<Mutex<Cached>> = LazyLock::new(|| Mutex::new(None));

fn param_pairs(f: &Fields<'_>, field: u32) -> Params {
    f.all_bytes(field)
        .filter_map(|p| {
            let p = Fields::parse(p);
            Some((p.str(1)?, p.str(2).unwrap_or_default()))
        })
        .collect()
}

fn parse_model(buf: &[u8]) -> Option<Model> {
    let f = Fields::parse(buf);
    let id = f
        .str(1)
        .filter(|s| !s.is_empty() && *s != "default")?
        .to_owned();
    let options: Vec<ParamOption> = f
        .all_bytes(29)
        .filter_map(|def| {
            let def = Fields::parse(def);
            let id = def.str(1).filter(|s| !s.is_empty())?.to_owned();
            let values = def.message(4).map_or_else(Vec::new, |v| {
                // Bool option groups (1), then enum option groups (2).
                [1, 2]
                    .into_iter()
                    .flat_map(|g| v.all_bytes(g).collect::<Vec<_>>())
                    .flat_map(|group| {
                        Fields::parse(group)
                            .all_bytes(1)
                            .filter_map(|opt| Fields::parse(opt).str(1).map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                    .collect()
            });
            Some(ParamOption { id, values })
        })
        .collect();
    let mut defaults = None;
    let mut names = Vec::new();
    for variant in f.all_bytes(30) {
        let v = Fields::parse(variant);
        let params = param_pairs(&v, 1);
        if params.is_empty() {
            continue;
        }
        if v.varint(4).unwrap_or(0) != 0 || v.varint(5).unwrap_or(0) != 0 {
            defaults.get_or_insert_with(|| params.clone());
        }
        for name in [v.str(11), v.str(9)]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
        {
            names.push(Spelling {
                name: name.to_owned(),
                params: params.clone(),
            });
        }
    }
    let defaults = defaults.unwrap_or_else(|| {
        options
            .iter()
            .filter_map(|ParamOption { id, values }| {
                let pick = match id.as_str() {
                    "thinking" if values.iter().any(|v| v == "true") => "true",
                    "effort" | "reasoning" if values.iter().any(|v| v == "high") => "high",
                    "effort" | "reasoning" => values.last()?,
                    _ => values.first()?,
                };
                Some((id.as_str(), pick))
            })
            .collect()
    });
    for alias in f.all_bytes(37).chain(f.all_bytes(36)) {
        if let Ok(name) = std::str::from_utf8(alias) {
            names.push(Spelling {
                name: name.to_owned(),
                params: defaults.clone(),
            });
        }
    }
    let display = f
        .str(17)
        .filter(|s| !s.is_empty())
        .unwrap_or(&id)
        .to_owned();
    Some(Model {
        id,
        display,
        options,
        defaults,
        names,
    })
}

async fn fetch(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    version: &str,
) -> Result<Vec<Model>> {
    let body = Msg::new()
        .bool(2, true)
        .bool(5, true)
        .bool(7, true)
        .finish();
    let resp = http
        .post(format!("{api_base}/aiserver.v1.AiService/AvailableModels"))
        .bearer_auth(token)
        .header("content-type", "application/proto")
        .header("connect-protocol-version", "1")
        .header("user-agent", "connect-es/1.6.1")
        .header("x-cursor-client-type", "cli")
        .header("x-cursor-client-version", version)
        .body(body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ByokError::Upstream {
            status: status.as_u16(),
            body,
            retry_after: None,
        });
    }
    let raw = resp.bytes().await?;
    Ok(Fields::parse(&raw)
        .all_bytes(2)
        .filter_map(parse_model)
        .collect())
}

/// The catalog, fetched at most every [`TTL`].
async fn catalog(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    version: &str,
) -> Result<Vec<Model>> {
    // ponytail: one process-wide catalog; per-account catalogs if plans ever differ.
    if let Some((at, models)) = CACHE.lock().expect("catalog lock").as_ref()
        && at.elapsed() < TTL
    {
        return Ok(models.clone());
    }
    let models = fetch(http, api_base, token, version).await?;
    *CACHE.lock().expect("catalog lock") = Some((Instant::now(), models.clone()));
    Ok(models)
}

/// A base model the account can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorModel {
    pub id: String,
    /// Human-readable name, e.g. `Claude Opus 5.5`.
    pub name: String,
}

impl From<Model> for CursorModel {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            name: m.display,
        }
    }
}

/// Every base model the account can use.
///
/// # Errors
///
/// Returns an error if the catalog cannot be fetched.
pub async fn list(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    version: &str,
) -> Result<Vec<CursorModel>> {
    let models = catalog(http, api_base, token, version).await?;
    Ok(models.into_iter().map(CursorModel::from).collect())
}

/// Resolve any accepted spelling of a model.
///
/// # Errors
///
/// Returns [`ByokError::UnsupportedModel`] if no model matches, or an error if
/// the catalog cannot be fetched.
pub async fn resolve(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    version: &str,
    name: &str,
) -> Result<Resolved> {
    let models = catalog(http, api_base, token, version).await?;
    resolve_in(&models, name).ok_or_else(|| ByokError::UnsupportedModel(name.to_owned()))
}

fn resolve_in(models: &[Model], name: &str) -> Option<Resolved> {
    let low = name.trim().to_ascii_lowercase();
    // `model[key=value,…]` overrides parameters explicitly.
    if let Some((head, tail)) = low.strip_suffix(']').and_then(|s| s.split_once('[')) {
        let mut hit = resolve_in(models, head)?;
        for pair in tail.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                hit.params.set(k.trim(), v.trim());
            }
        }
        return Some(hit);
    }
    let index: HashMap<String, (&Model, &Params)> = models
        .iter()
        .flat_map(|m| {
            std::iter::once((m.id.to_ascii_lowercase(), (m, &m.defaults))).chain(
                m.names
                    .iter()
                    .map(move |s| (s.name.to_ascii_lowercase(), (m, &s.params))),
            )
        })
        .rev() // first spelling wins on collision
        .collect();
    let exact = |s: &str| {
        index.get(s).map(|(m, p)| Resolved {
            id: m.id.clone(),
            params: (*p).clone(),
        })
    };
    if let Some(hit) = exact(&low) {
        return Some(hit);
    }
    // `<model>-<value>-<value>`: the longest known prefix, then parameter values.
    let parts: Vec<&str> = low.split('-').collect();
    (1..parts.len()).rev().find_map(|cut| {
        let (model, params) = index.get(&parts[..cut].join("-"))?;
        let mut hit = Resolved {
            id: model.id.clone(),
            params: (*params).clone(),
        };
        for tok in &parts[cut..] {
            match *tok {
                "thinking" | "think" => hit.params.set("thinking", "true"),
                "nothinking" | "nonthinking" => hit.params.set("thinking", "false"),
                "fast" => hit.params.set("fast", "true"),
                tok => hit.params.set(model.option_taking(tok)?, tok),
            }
        }
        Some(hit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pairs: &[(&str, &str)]) -> Params {
        pairs.iter().copied().collect()
    }

    fn option(id: &str, values: &[&str]) -> ParamOption {
        ParamOption {
            id: id.into(),
            values: values.iter().map(|&v| v.into()).collect(),
        }
    }

    fn spelling(name: &str, params: &[(&str, &str)]) -> Spelling {
        Spelling {
            name: name.into(),
            params: p(params),
        }
    }

    fn opus() -> Model {
        Model {
            id: "claude-opus-5-5".into(),
            display: "Claude Opus 5.5".into(),
            options: vec![
                option("effort", &["low", "high"]),
                option("fast", &["false", "true"]),
            ],
            defaults: p(&[("effort", "high"), ("fast", "false")]),
            names: vec![
                spelling(
                    "claude-opus-5-5-low-fast",
                    &[("effort", "low"), ("fast", "true")],
                ),
                spelling("opus", &[("effort", "high"), ("fast", "false")]),
            ],
        }
    }

    #[test]
    fn resolves_ids_variants_and_aliases() {
        let m = [opus()];
        assert_eq!(
            resolve_in(&m, "claude-opus-5-5").unwrap().params,
            p(&[("effort", "high"), ("fast", "false")])
        );
        assert_eq!(
            resolve_in(&m, "Claude-Opus-5-5-Low-Fast").unwrap().params,
            p(&[("effort", "low"), ("fast", "true")])
        );
        assert_eq!(resolve_in(&m, "opus").unwrap().id, "claude-opus-5-5");
    }

    #[test]
    fn resolves_parameter_tokens_and_overrides() {
        let m = [opus()];
        assert_eq!(
            resolve_in(&m, "claude-opus-5-5-high-fast").unwrap().params,
            p(&[("effort", "high"), ("fast", "true")])
        );
        assert_eq!(
            resolve_in(&m, "opus[effort=low]").unwrap().params,
            p(&[("effort", "low"), ("fast", "false")])
        );
        assert!(resolve_in(&m, "claude-opus-5-5-bogus").is_none());
        assert!(resolve_in(&m, "gpt-9").is_none());
    }

    #[test]
    fn parses_catalog_entries() {
        let opt = |v: &str| Msg::new().msg(1, &Msg::new().str(1, v));
        let values = Msg::new().msg(2, &opt("low")).msg(2, &opt("high"));
        let def = Msg::new().str(1, "effort").msg(4, &values);
        let variant = Msg::new()
            .msg(1, &Msg::new().str(1, "effort").str(2, "low"))
            .varint(4, 1)
            .str(11, "m-low");
        let raw = Msg::new()
            .str(1, "m")
            .msg(29, &def)
            .msg(30, &variant)
            .str(37, "alias")
            .finish();
        let m = parse_model(&raw).unwrap();
        assert_eq!(m.options, [option("effort", &["low", "high"])]);
        assert_eq!(m.defaults, p(&[("effort", "low")]));
        assert_eq!(
            m.names.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["m-low", "alias"]
        );
        assert_eq!(m.display, "m");
        assert!(parse_model(&Msg::new().str(1, "default").finish()).is_none());
    }
}
