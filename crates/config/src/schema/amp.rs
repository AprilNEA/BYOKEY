use serde::{Deserialize, Serialize};

/// `AmpCode` 管理代理配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AmpConfig {
    /// 设置后，byokey 进入"共享代理"模式：
    /// 客户端的 Authorization / X-Api-Key 头会被剥离，
    /// 改为注入此 key（同时设置 `Authorization: Bearer {key}` 和 `X-Api-Key: {key}`）。
    /// 不设置则保持 BYOK 透传行为（默认）。
    #[serde(default)]
    pub upstream_key: Option<String>,
    /// 拦截 `getUserFreeTierStatus` 响应，将 `canUseAmpFree` 和
    /// `isDailyGrantEnabled` 改为 `false`，隐藏免费层提示（默认关闭）。
    #[serde(default)]
    pub hide_free_tier: bool,
}

#[cfg(test)]
mod tests {
    use crate::schema::Config;

    #[test]
    fn test_default_amp_upstream_key_is_none() {
        let c = Config::default();
        assert!(c.amp.upstream_key.is_none());
    }

    #[test]
    fn test_from_yaml_amp_upstream_key() {
        let yaml = r#"
amp:
  upstream_key: "amp-key-xxx"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.amp.upstream_key.as_deref(), Some("amp-key-xxx"));
    }

    #[test]
    fn test_from_yaml_amp_defaults_when_omitted() {
        let c = Config::from_yaml("port: 1234").unwrap();
        assert!(c.amp.upstream_key.is_none());
    }
}
