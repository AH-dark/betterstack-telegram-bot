use std::sync::Arc;
use std::time::Duration;

use betterstack_telegram_bot::betterstack::client::BetterStackClient;
use betterstack_telegram_bot::betterstack::webhook::WebhookAppState;
use betterstack_telegram_bot::config::Config;
use betterstack_telegram_bot::logging;
use betterstack_telegram_bot::server;
use betterstack_telegram_bot::storage::redis::RedisStore;
use betterstack_telegram_bot::storage::IncidentStore;
use betterstack_telegram_bot::telegram::dispatch::{build_dispatcher, DispatcherDeps};
use betterstack_telegram_bot::telegram::{self, TeloxideNotifier};
use clap::Parser;
use teloxide::prelude::*;
use teloxide::update_listeners::webhooks;

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
    ) as Arc<dyn IncidentStore>;

    let bot = Bot::new(config.bot_token.expose().to_string());
    let betterstack_client = BetterStackClient::new(
        config.betterstack_api_base.clone(),
        config.betterstack_api_token.expose().to_string(),
    );
    let notifier =
        Arc::new(TeloxideNotifier::new(bot.clone(), config.chat_id)) as Arc<dyn telegram::Notifier>;
    let secret_token = server::resolve_secret_token(&config);

    server::register_webhook(&bot, &config, &secret_token).await?;

    let webhook_url = config
        .public_url
        .join(config.telegram_webhook_path.trim_start_matches('/'))?;
    let options = webhooks::Options::new(config.bind, webhook_url)
        .path(config.telegram_webhook_path.clone())
        .secret_token(secret_token.clone());

    let (listener, stop_flag, teloxide_router) = webhooks::axum_no_setup(options);

    let webhook_state = WebhookAppState {
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
