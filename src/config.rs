use clap::Parser;
use std::convert::Infallible;
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use url::Url;

/// Wrapper that redacts the inner value in Debug/Display output.
#[derive(Clone)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = self.expose();
        write!(f, "***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = self.expose();
        write!(f, "***")
    }
}

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Secret(s)
    }
}

impl FromStr for Secret {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Secret(s.to_string()))
    }
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "betterstack-telegram-bot",
    about = "Better Stack incident -> Telegram bot"
)]
pub struct Config {
    /// Telegram bot token
    #[arg(long, env = "TELEGRAM_BOT_TOKEN")]
    pub bot_token: Secret,

    /// Target group chat ID (negative for supergroups)
    #[arg(long, env = "TELEGRAM_CHAT_ID")]
    pub chat_id: i64,

    /// Externally reachable base URL for Telegram webhook registration
    #[arg(long, env = "PUBLIC_URL")]
    pub public_url: Url,

    /// Path teloxide listens on for Telegram updates
    #[arg(
        long,
        env = "TELEGRAM_WEBHOOK_PATH",
        default_value = "/telegram/webhook"
    )]
    pub telegram_webhook_path: String,

    /// Telegram secret_token header check (random per boot if unset)
    #[arg(long, env = "TELEGRAM_SECRET_TOKEN")]
    pub telegram_secret_token: Option<Secret>,

    /// Path for inbound Better Stack incident webhooks
    #[arg(
        long,
        env = "BETTERSTACK_WEBHOOK_PATH",
        default_value = "/webhooks/betterstack"
    )]
    pub betterstack_webhook_path: String,

    /// Shared secret required on inbound Better Stack webhooks (X-Webhook-Secret header)
    #[arg(long, env = "BETTERSTACK_WEBHOOK_SECRET")]
    pub betterstack_webhook_secret: Secret,

    /// Bearer token for Better Stack Uptime API
    #[arg(long, env = "BETTERSTACK_API_TOKEN")]
    pub betterstack_api_token: Secret,

    /// Better Stack API base URL
    #[arg(
        long,
        env = "BETTERSTACK_API_BASE",
        default_value = "https://uptime.betterstack.com"
    )]
    pub betterstack_api_base: Url,

    /// Redis connection URL
    #[arg(long, env = "REDIS_URL")]
    pub redis_url: String,

    /// Redis key prefix for namespacing
    #[arg(long, env = "REDIS_KEY_PREFIX", default_value = "bstg")]
    pub redis_key_prefix: String,

    /// TTL in seconds for incident records in Redis
    #[arg(long, env = "INCIDENT_TTL_SECS", default_value_t = 2_592_000)]
    pub incident_ttl_secs: u64,

    /// Listen address
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

    /// Log format: json or pretty
    #[arg(long, env = "LOG_FORMAT", default_value = "json")]
    pub log_format: LogFormat,

    /// Log level / EnvFilter directive
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum LogFormat {
    Json,
    Pretty,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const CONFIG_ENV_VARS: &[&str] = &[
        "TELEGRAM_BOT_TOKEN",
        "TELEGRAM_CHAT_ID",
        "PUBLIC_URL",
        "TELEGRAM_WEBHOOK_PATH",
        "TELEGRAM_SECRET_TOKEN",
        "BETTERSTACK_WEBHOOK_PATH",
        "BETTERSTACK_WEBHOOK_SECRET",
        "BETTERSTACK_API_TOKEN",
        "BETTERSTACK_API_BASE",
        "REDIS_URL",
        "REDIS_KEY_PREFIX",
        "INCIDENT_TTL_SECS",
        "BIND_ADDR",
        "LOG_FORMAT",
        "RUST_LOG",
    ];

    #[test]
    fn secret_debug_redacts_value() {
        let s = Secret("super-secret-token-12345".to_string());
        let debug_output = format!("{:?}", s);
        assert_eq!(debug_output, "***");
        assert!(!debug_output.contains("super-secret-token-12345"));
    }

    #[test]
    fn secret_display_redacts_value() {
        let s = Secret("my-api-key".to_string());
        let display_output = format!("{}", s);
        assert_eq!(display_output, "***");
        assert!(!display_output.contains("my-api-key"));
    }

    #[test]
    fn secret_expose_returns_value() {
        let s = Secret("actual-value".to_string());
        assert_eq!(s.expose(), "actual-value");
    }

    #[test]
    fn config_debug_does_not_leak_secrets() {
        let config = Config {
            bot_token: Secret("bot-token-secret".to_string()),
            chat_id: -100123,
            public_url: "https://bot.example.com".parse().expect("valid URL"),
            telegram_webhook_path: "/telegram/webhook".to_string(),
            telegram_secret_token: Some(Secret("telegram-secret-token".to_string())),
            betterstack_webhook_path: "/webhooks/betterstack".to_string(),
            betterstack_webhook_secret: Secret("webhook-secret".to_string()),
            betterstack_api_token: Secret("api-token-secret".to_string()),
            betterstack_api_base: "https://uptime.betterstack.com".parse().expect("valid URL"),
            redis_url: "redis://localhost:6379/0".to_string(),
            redis_key_prefix: "bstg".to_string(),
            incident_ttl_secs: 2_592_000,
            bind: "0.0.0.0:8080".parse().expect("valid socket address"),
            log_format: LogFormat::Json,
            log_level: "info".to_string(),
        };

        let debug = format!("{:?}", config);
        assert!(!debug.contains("bot-token-secret"));
        assert!(!debug.contains("telegram-secret-token"));
        assert!(!debug.contains("webhook-secret"));
        assert!(!debug.contains("api-token-secret"));
    }

    #[test]
    fn env_vars_populate_config_fields() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        for name in CONFIG_ENV_VARS {
            env::remove_var(name);
        }

        env::set_var("TELEGRAM_BOT_TOKEN", "bot-token-from-env");
        env::set_var("TELEGRAM_CHAT_ID", "-100987654321");
        env::set_var("PUBLIC_URL", "https://bot.example.com");
        env::set_var("TELEGRAM_WEBHOOK_PATH", "/telegram/custom");
        env::set_var("TELEGRAM_SECRET_TOKEN", "telegram-secret-from-env");
        env::set_var("BETTERSTACK_WEBHOOK_PATH", "/betterstack/custom");
        env::set_var("BETTERSTACK_WEBHOOK_SECRET", "webhook-secret-from-env");
        env::set_var("BETTERSTACK_API_TOKEN", "api-token-from-env");
        env::set_var("BETTERSTACK_API_BASE", "https://uptime.example.test");
        env::set_var("REDIS_URL", "redis://localhost:6379/1");
        env::set_var("REDIS_KEY_PREFIX", "custom");
        env::set_var("INCIDENT_TTL_SECS", "60");
        env::set_var("BIND_ADDR", "127.0.0.1:9000");
        env::set_var("LOG_FORMAT", "pretty");
        env::set_var("RUST_LOG", "debug");

        let config = Config::try_parse_from(["betterstack-telegram-bot"]).expect("config parses");

        assert_eq!(config.bot_token.expose(), "bot-token-from-env");
        assert_eq!(config.chat_id, -100987654321);
        assert_eq!(config.public_url.as_str(), "https://bot.example.com/");
        assert_eq!(config.telegram_webhook_path, "/telegram/custom");
        assert_eq!(
            config.telegram_secret_token.as_ref().map(Secret::expose),
            Some("telegram-secret-from-env")
        );
        assert_eq!(config.betterstack_webhook_path, "/betterstack/custom");
        assert_eq!(
            config.betterstack_webhook_secret.expose(),
            "webhook-secret-from-env"
        );
        assert_eq!(config.betterstack_api_token.expose(), "api-token-from-env");
        assert_eq!(
            config.betterstack_api_base.as_str(),
            "https://uptime.example.test/"
        );
        assert_eq!(config.redis_url, "redis://localhost:6379/1");
        assert_eq!(config.redis_key_prefix, "custom");
        assert_eq!(config.incident_ttl_secs, 60);
        assert_eq!(
            config.bind,
            "127.0.0.1:9000".parse().expect("valid socket address")
        );
        assert_eq!(config.log_format, LogFormat::Pretty);
        assert_eq!(config.log_level, "debug");

        for name in CONFIG_ENV_VARS {
            env::remove_var(name);
        }
    }
}
