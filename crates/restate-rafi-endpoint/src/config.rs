//! Endpoint configuration: a file (JSON, YAML or TOML) merged with
//! `RESTATE_RAFI_`-prefixed environment variables (`__` separates levels).

use std::path::Path;

use anyhow::{Context, Result, bail};
use figment::{
    Figment,
    providers::{Env, Format, Json, Toml, Yaml},
};
use rafi_drand::DrandConfig;
use rafi_store::{Db, DrawWriter, PurgeAfter};
use restate_config::{EndpointConfig, ServiceOptionsConfig};
use serde::{Deserialize, Serialize};

/// The environment variable prefix (e.g. `RESTATE_RAFI_DATABASE__URL`).
pub const ENV_PREFIX: &str = "RESTATE_RAFI_";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Listener and request identity verification.
    pub endpoint: EndpointConfig,
    /// Restate policy per service.
    pub services: ServicesConfig,
    /// Beacon relays and timing.
    pub drand: DrandConfig,
    /// The Rafi app's database (SQLite or Postgres), where the draw state is written.
    pub database: DatabaseConfig,
}

/// The Rafi app's database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    /// The same database the app uses (its `RAFI_DATABASE_URL`): `sqlite://…`
    /// or `postgres://…`.
    pub url: String,
    /// Days after a raffle finishes until non-winners are erased. Set it to the
    /// app's `RAFI_RETENTION_DAYS`: whoever stores the finished state dates it.
    pub retention_nonwinners_days: i64,
    /// Days after a raffle finishes until winners are erased (the app's
    /// `RAFI_RETENTION_WINNERS_DAYS`).
    pub retention_winners_days: i64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "sqlite://rafi.db".into(),
            retention_nonwinners_days: 30,
            retention_winners_days: 90,
        }
    }
}

impl DatabaseConfig {
    /// When personal data of a finished raffle is erased.
    pub fn purge_after(&self) -> PurgeAfter {
        PurgeAfter::days(self.retention_nonwinners_days, self.retention_winners_days)
    }

    /// Where the drawing engine writes draw states: the app's database, opened
    /// lazily. The app owns the schema (and creates the SQLite file); until it
    /// has, writes fail and Restate retries them.
    pub fn draw_store(&self) -> Result<DrawWriter> {
        if self.url.trim().is_empty() {
            bail!("database.url is required");
        }
        let db = Db::connect_lazy(&self.url)
            .with_context(|| format!("invalid database.url {:?}", self.url))?;
        Ok(DrawWriter::new(db, self.purge_after()))
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServicesConfig {
    /// Policy for the `Drawing` virtual object.
    pub drawing: ServiceOptionsConfig,
}

impl Config {
    /// Loads the optional file, then environment overrides.
    pub fn load(file: Option<&Path>) -> Result<Self> {
        Self::figment(file)?
            .extract()
            .context("invalid configuration")
    }

    pub fn figment(file: Option<&Path>) -> Result<Figment> {
        let mut figment = Figment::new();
        if let Some(path) = file {
            if !path.exists() {
                bail!("config file not found: {}", path.display());
            }
            figment = match path.extension().and_then(|s| s.to_str()) {
                Some("toml") => figment.merge(Toml::file(path)),
                Some("json") => figment.merge(Json::file(path)),
                Some("yaml" | "yml") => figment.merge(Yaml::file(path)),
                _ => bail!("unsupported config file format; use .toml, .json, .yaml or .yml"),
            };
        }
        Ok(figment.merge(Env::prefixed(ENV_PREFIX).split("__")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::result_large_err)] // figment::Jail returns figment::Error

    use super::*;

    #[test]
    fn database_url_is_required() {
        let cfg = DatabaseConfig {
            url: " ".into(),
            ..DatabaseConfig::default()
        };
        assert!(cfg.draw_store().is_err());
    }

    #[tokio::test]
    async fn opening_does_not_create_the_file() {
        let path = std::env::temp_dir().join(format!("rafi-endpoint-{}.db", std::process::id()));
        DatabaseConfig {
            url: format!("sqlite://{}", path.display()),
            ..DatabaseConfig::default()
        }
        .draw_store()
        .unwrap();
        assert!(
            !path.exists(),
            "the app creates the database, not the drawing engine"
        );
    }

    #[test]
    fn defaults_listen_on_9080_with_real_drand() {
        figment::Jail::expect_with(|_| {
            let cfg = Config::load(None).expect("defaults");
            assert_eq!(cfg.endpoint.listener.port(), 9080);
            assert!(!cfg.drand.mock);
            assert_eq!(cfg.drand.lead_secs, 6);
            assert_eq!(cfg.database.url, "sqlite://rafi.db");
            assert_eq!(cfg.database.retention_nonwinners_days, 30);
            assert_eq!(cfg.database.retention_winners_days, 90);
            Ok(())
        });
    }

    #[test]
    fn file_and_environment_merge() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "rafi.toml",
                r#"
                [database]
                url = "sqlite:///data/rafi.db"
                retention_winners_days = 120

                [services.drawing]
                inactivity_timeout = "5m"
                "#,
            )?;
            jail.set_env("RESTATE_RAFI_DRAND__LEAD_SECS", "9");
            jail.set_env(
                "RESTATE_RAFI_DRAND__RELAYS",
                "[https://a.example, https://b.example]",
            );
            jail.set_env("RESTATE_RAFI_ENDPOINT__LISTENER", "127.0.0.1:9999");
            jail.set_env(
                "RESTATE_RAFI_ENDPOINT__IDENTITY_KEYS",
                "publickeyv1_a, publickeyv1_b",
            );
            let cfg = Config::load(Some(Path::new("rafi.toml"))).expect("config");
            assert_eq!(cfg.database.url, "sqlite:///data/rafi.db");
            assert_eq!(cfg.database.purge_after().winners_secs, 120 * 86_400);
            assert_eq!(cfg.database.retention_nonwinners_days, 30);
            assert_eq!(cfg.drand.lead_secs, 9);
            assert_eq!(cfg.drand.relays, ["https://a.example", "https://b.example"]);
            assert!(cfg.drand.validate().is_ok(), "unprefixed relays are v1");
            assert_eq!(cfg.endpoint.listener.port(), 9999);
            assert_eq!(
                cfg.endpoint.identity_keys,
                ["publickeyv1_a", "publickeyv1_b"]
            );
            assert!(cfg.services.drawing.inactivity_timeout.is_some());
            Ok(())
        });
    }

    #[test]
    fn unknown_keys_are_rejected() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("RESTATE_RAFI_DATABSE__URL", "typo");
            assert!(Config::load(None).is_err());
            Ok(())
        });
    }
}
