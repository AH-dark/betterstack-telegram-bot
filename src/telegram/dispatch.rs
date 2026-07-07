use std::sync::Arc;

use teloxide::adaptors::Throttle;
use teloxide::dispatching::{DefaultKey, UpdateFilterExt, UpdateHandler};
use teloxide::prelude::*;
use teloxide::requests::RequesterExt;
use teloxide::types::Update;

use super::{handler, Notifier};
use crate::betterstack::client::BetterStackClient;
use crate::storage::IncidentStore;

pub type HandlerError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Dependencies injected into the dispatcher.
#[derive(Clone)]
pub struct DispatcherDeps {
    pub store: Arc<dyn IncidentStore>,
    pub notifier: Arc<dyn Notifier>,
    pub betterstack_client: BetterStackClient,
}

/// Build the teloxide callback_query handler branch.
pub fn build_handler() -> UpdateHandler<HandlerError> {
    Update::filter_callback_query().endpoint(handler::callback_handler)
}

/// Build a throttled teloxide dispatcher with application dependencies.
pub fn build_dispatcher(
    bot: Bot,
    deps: DispatcherDeps,
) -> Dispatcher<Throttle<Bot>, HandlerError, DefaultKey> {
    Dispatcher::builder(bot.throttle(Default::default()), build_handler())
        .dependencies(dptree::deps![
            deps.store,
            deps.notifier,
            deps.betterstack_client
        ])
        .enable_ctrlc_handler()
        .build()
}
