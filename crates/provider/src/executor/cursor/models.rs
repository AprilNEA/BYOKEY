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

/// One model and every spelling that selects it.
#[derive(Debug, Clone)]
struct Model {
    id: String,
    /// Parameter id → allowed values, in display order.
    options: Vec<(String, Vec<String>)>,
    /// Parameters sent when the caller names only the model.
    defaults: Vec<(String, String)>,
    /// `(name, parameters)` for each variant, alias and legacy slug.
    names: Vec<(String, Vec<(String, String)>)>,
}

/// A resolved model: what goes on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub params: Vec<(String, String)>,
}

impl Resolved {
    fn set(&mut self, key: &str, value: &str) {
        match self.params.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => value.clone_into(&mut slot.1),
            None => self.params.push((key.to_owned(), value.to_owned())),
        }
    }
}

type Params = Vec<(String, String)>;

/// The catalog and when it was fetched.
type Cached = Option<(Instant, Vec<Model>)>;

static CACHE: LazyLock<Mutex<Cached>> = LazyLock::new(|| Mutex::new(None));

fn param_pairs(f: &Fields<'_>, field: u32) -> Vec<(String, String)> {
    f.all_bytes(field)
        .filter_map(|p| {
            let p = Fields::parse(p);
            Some((
                p.str(1)?.to_owned(),
                p.str(2).unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

fn parse_model(buf: &[u8]) -> Option<Model> {
    let f = Fields::parse(buf);
    let id = f
        .str(1)
        .filter(|s| !s.is_empty() && *s != "default")?
        .to_owned();
    let options: Vec<(String, Vec<String>)> = f
        .all_bytes(29)
        .filter_map(|def| {
            let def = Fields::parse(def);
            let pid = def.str(1).filter(|s| !s.is_empty())?.to_owned();
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
            Some((pid, values))
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
            names.push((name.to_owned(), params.clone()));
        }
    }
    let defaults = defaults.unwrap_or_else(|| {
        options
            .iter()
            .filter_map(|(pid, values)| {
                let pick = match pid.as_str() {
                    "thinking" if values.iter().any(|v| v == "true") => "true",
                    "effort" | "reasoning" if values.iter().any(|v| v == "high") => "high",
                    "effort" | "reasoning" => values.last()?,
                    _ => values.first()?,
                };
                Some((pid.clone(), pick.to_owned()))
            })
            .collect()
    });
    for alias in f.all_bytes(37).chain(f.all_bytes(36)) {
        if let Ok(name) = std::str::from_utf8(alias) {
            names.push((name.to_owned(), defaults.clone()));
        }
    }
    Some(Model {
        id,
        options,
        defaults,
        names,
    })
}

async fn fetch(
    http: &wreq::Client,
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
    http: &wreq::Client,
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

/// Every model name the account can use: base ids first, then variants.
///
/// # Errors
///
/// Returns an error if the catalog cannot be fetched.
pub async fn names(
    http: &wreq::Client,
    api_base: &str,
    token: &str,
    version: &str,
) -> Result<Vec<String>> {
    let models = catalog(http, api_base, token, version).await?;
    let mut out: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
    for m in &models {
        for (name, _) in &m.names {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
    }
    Ok(out)
}

/// Resolve any accepted spelling of a model.
///
/// # Errors
///
/// Returns [`ByokError::UnsupportedModel`] if no model matches, or an error if
/// the catalog cannot be fetched.
pub async fn resolve(
    http: &wreq::Client,
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
                hit.set(k.trim(), v.trim());
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
                    .map(move |(n, p)| (n.to_ascii_lowercase(), (m, p))),
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
                "thinking" | "think" => hit.set("thinking", "true"),
                "nothinking" | "nonthinking" => hit.set("thinking", "false"),
                "fast" => hit.set("fast", "true"),
                tok => {
                    let (pid, _) = model
                        .options
                        .iter()
                        .find(|(_, vs)| vs.iter().any(|v| v == tok))?;
                    hit.set(pid, tok);
                }
            }
        }
        Some(hit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn opus() -> Model {
        Model {
            id: "claude-opus-5-5".into(),
            options: vec![
                ("effort".into(), vec!["low".into(), "high".into()]),
                ("fast".into(), vec!["false".into(), "true".into()]),
            ],
            defaults: p(&[("effort", "high"), ("fast", "false")]),
            names: vec![
                (
                    "claude-opus-5-5-low-fast".into(),
                    p(&[("effort", "low"), ("fast", "true")]),
                ),
                ("opus".into(), p(&[("effort", "high"), ("fast", "false")])),
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
        assert_eq!(
            m.options,
            vec![(
                "effort".to_owned(),
                vec!["low".to_owned(), "high".to_owned()]
            )]
        );
        assert_eq!(m.defaults, p(&[("effort", "low")]));
        assert_eq!(
            m.names.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["m-low", "alias"]
        );
        assert!(parse_model(&Msg::new().str(1, "default").finish()).is_none());
    }
}
