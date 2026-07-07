pub mod betterstack;
mod config;
pub mod domain;
pub mod error;
mod logging;
pub mod render;
mod server;
pub mod storage;
pub mod telegram;

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use config::Config;
use teloxide::prelude::*;
use teloxide::update_listeners::webhooks;

use betterstack::client::BetterStackClient;
use storage::redis::RedisStore;
use telegram::dispatch::{build_dispatcher, DispatcherDeps};
use telegram::TeloxideNotifier;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    logging::init_logging(&config.log_format, &config.log_level);
    tracing::info!("betterstack-telegram-bot starting");

    let store = Arc::new(
        RedisStore::new(
            &config.redis_url,
            config.redis_key_prefix.clone(),
            Duration::from_secs(config.incident_ttl_secs),
        )
        .await?,
    ) as Arc<dyn storage::IncidentStore>;

    let bot = Bot::new(config.bot_token.expose().to_string());
    let betterstack_client = BetterStackClient::new(
        config.betterstack_api_base.clone(),
        config.betterstack_api_token.expose().to_string(),
    );
    let notifier =
        Arc::new(TeloxideNotifier::new(bot.clone(), config.chat_id)) as Arc<dyn telegram::Notifier>;

    server::register_webhook(&bot, &config).await?;

    let webhook_url = config
        .public_url
        .join(config.telegram_webhook_path.trim_start_matches('/'))?;
    let mut options =
        webhooks::Options::new(config.bind, webhook_url).path(config.telegram_webhook_path.clone());
    if let Some(secret) = &config.telegram_secret_token {
        options = options.secret_token(secret.expose().to_string());
    }

    let (listener, stop_flag, teloxide_router) = webhooks::axum_no_setup(options);

    let webhook_state = betterstack::webhook::WebhookAppState {
        store: store.clone(),
        notifier: notifier.clone(),
        webhook_secret: config.betterstack_webhook_secret.expose().to_string(),
    };
    let app = server::build_router(
        teloxide_router,
        webhook_state,
        &config.betterstack_webhook_path,
    );

    let deps = DispatcherDeps {
        store,
        notifier,
        betterstack_client,
    };
    let mut dispatcher = build_dispatcher(bot, deps);

    tracing::info!(bind = %config.bind, "serving");
    let tcp_listener = tokio::net::TcpListener::bind(config.bind).await?;
    let axum_server = axum::serve(tcp_listener, app).with_graceful_shutdown(stop_flag);

    tokio::select! {
        result = axum_server => {
            if let Err(err) = result {
                tracing::error!(error = %err, "axum server error");
            }
        }
        _ = dispatcher.dispatch_with_listener(
            listener,
            LoggingErrorHandler::with_custom_text("dispatcher error"),
        ) => {}
    }

    tracing::info!("betterstack-telegram-bot stopped");
    Ok(())
}
