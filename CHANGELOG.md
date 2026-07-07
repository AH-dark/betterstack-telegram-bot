# Changelog

## [0.2.0](https://github.com/AH-dark/betterstack-telegram-bot/compare/v0.1.0...v0.2.0) (2026-07-07)


### Features

* add AppError type with thiserror and axum IntoResponse ([67d3ad4](https://github.com/AH-dark/betterstack-telegram-bot/commit/67d3ad46eed5c7c96b2e81847172c80242b6675e))
* add Better Stack API client with Bearer auth, 409 idempotency, retry, wiremock tests ([c3e74ec](https://github.com/AH-dark/betterstack-telegram-bot/commit/c3e74ecd67537f459c60ea28b7226ee03b68512a))
* add Better Stack webhook handler with secret check, dedup, state machine, send/edit ([120ace0](https://github.com/AH-dark/betterstack-telegram-bot/commit/120ace05993ac536def600f650c5077a396485c1))
* add Better Stack webhook payload types with serde tolerance tests ([59fdb67](https://github.com/AH-dark/betterstack-telegram-bot/commit/59fdb6720d018f214ffe430e93877e09f4c518b4))
* add callback codec with 64-byte guard and round-trip tests ([069376e](https://github.com/AH-dark/betterstack-telegram-bot/commit/069376e3e67c131ed0fd753309d273dc9931c4b3))
* add config module with Secret newtype and clap parser ([f919402](https://github.com/AH-dark/betterstack-telegram-bot/commit/f919402eded42feae9e20c3838d1ecbb2196a7ef))
* add incident domain model and state machine with full transition table tests ([ca0b07a](https://github.com/AH-dark/betterstack-telegram-bot/commit/ca0b07a58c12140cd6fe65077925a93383a9eccc))
* add incident message renderer with HTML escaping and status-based keyboard ([523a3cb](https://github.com/AH-dark/betterstack-telegram-bot/commit/523a3cb625e37c0ae819f1c844c95b09d673aaf3))
* add IncidentStore trait and MemoryStore with async tests ([807b35d](https://github.com/AH-dark/betterstack-telegram-bot/commit/807b35dc053a73c0b77fc12eca182050d534491c))
* add logging init module with JSON/pretty tracing-subscriber ([d013d74](https://github.com/AH-dark/betterstack-telegram-bot/commit/d013d747abc36ee9b21e9471d6b6a6dfc63a2838))
* add Notifier trait with TeloxideNotifier and FakeNotifier recording fake ([ff96084](https://github.com/AH-dark/betterstack-telegram-bot/commit/ff96084d88db3a906878a4410f511ce4e2286246))
* add RedisStore with ConnectionManager, HSET/HGETALL, SET NX EX, Lua lock release ([1306361](https://github.com/AH-dark/betterstack-telegram-bot/commit/130636109658039af3089d2ad2d8e631106d63ef))
* add Telegram dispatcher and callback handler with actor attribution ([46642e5](https://github.com/AH-dark/betterstack-telegram-bot/commit/46642e5c41a1a883f4d677cb3b04fa96e7b453a2))
* wire full server with axum router, healthz, graceful shutdown, and main entry point ([71a0e4b](https://github.com/AH-dark/betterstack-telegram-bot/commit/71a0e4bd797394e3319e677a0903e55e8380b387))


### Bug Fixes

* harden concurrency and security per review ([f7d3f52](https://github.com/AH-dark/betterstack-telegram-bot/commit/f7d3f5246e31480e030b5cc8cb63181e51cb0656))
