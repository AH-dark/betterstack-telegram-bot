# betterstack-telegram-bot

A stateless, horizontally scalable Rust service that bridges [Better Stack Uptime](https://betterstack.com/uptime) incidents to a Telegram group with two-way sync.

## Features

- Receives Better Stack incident webhooks and posts them to a configured Telegram group
- Inline keyboard buttons: **Acknowledge** and **Resolve** — clicking calls the Better Stack API and edits the message in place
- Status changes originating in Better Stack (ack/resolve via their UI) are reflected back into the same Telegram message
- Idempotent webhook handling (dedup via Redis `SET NX EX`)
- Webhook mode (not polling) — multiple replicas can run behind a load balancer
- All state in Redis — replicas are interchangeable, restarts lose nothing
- Structured JSON logging via `tracing`

## Prerequisites

- A Telegram bot token (from [@BotFather](https://t.me/BotFather))
- The bot added to your target Telegram group as an admin (to send and edit messages)
- A Better Stack Uptime account with outgoing webhooks configured
- A Redis instance (any version supporting `SET NX EX`)
- An HTTPS endpoint reachable by both Telegram and Better Stack (e.g. behind nginx/Caddy/ingress)

## Quick Start

```bash
docker run --rm \
  -e TELEGRAM_BOT_TOKEN=your-bot-token \
  -e TELEGRAM_CHAT_ID=-100123456789 \
  -e PUBLIC_URL=https://bot.example.com \
  -e BETTERSTACK_WEBHOOK_SECRET=your-shared-secret \
  -e BETTERSTACK_API_TOKEN=your-betterstack-api-token \
  -e REDIS_URL=redis://:password@redis-host:6379/0 \
  -p 127.0.0.1:8080:8080 \
  ghcr.io/ah-dark/betterstack-telegram-bot:latest
```

## Configuration

All configuration is via environment variables (or CLI flags — run with `--help` for details).

| Environment Variable | Default | Required | Description |
|---|---|---|---|
| `TELEGRAM_BOT_TOKEN` | — |  | Telegram bot token from BotFather |
| `TELEGRAM_CHAT_ID` | — |  | Target group chat ID (negative for supergroups, e.g. `-100123456789`) |
| `PUBLIC_URL` | — |  | Externally reachable base URL (e.g. `https://bot.example.com`) — used to register the Telegram webhook |
| `TELEGRAM_WEBHOOK_PATH` | `/telegram/webhook` | | Path teloxide listens on for Telegram updates |
| `TELEGRAM_SECRET_TOKEN` | random per boot | | Telegram `secret_token` header check for webhook security |
| `BETTERSTACK_WEBHOOK_PATH` | `/webhooks/betterstack` | | Path for inbound Better Stack incident webhooks |
| `BETTERSTACK_WEBHOOK_SECRET` | — |  | Shared secret sent in `X-Webhook-Secret` header by Better Stack |
| `BETTERSTACK_API_TOKEN` | — |  | Bearer token for Better Stack Uptime API (acknowledge/resolve) |
| `BETTERSTACK_API_BASE` | `https://uptime.betterstack.com` | | Better Stack API base URL (override for testing) |
| `REDIS_URL` | — |  | Redis connection URL, e.g. `redis://:pass@host:6379/0` |
| `REDIS_KEY_PREFIX` | `bstg` | | Prefix for all Redis keys (useful for multi-tenant setups) |
| `INCIDENT_TTL_SECS` | `2592000` (30 days) | | TTL for incident records in Redis |
| `BIND_ADDR` | `0.0.0.0:8080` | | Listen address |
| `LOG_FORMAT` | `json` | | Log format: `json` (production) or `pretty` (local dev) |
| `RUST_LOG` | `info` | | Log level / [EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) directive |

## Better Stack Setup

### 1. Create an Outgoing Webhook

In Better Stack: **Integrations → Outgoing Webhooks → New Webhook**

- **URL**: `https://your-domain.com/webhooks/betterstack`
- **Method**: POST
- **Content-Type**: application/json
- **Custom Header**: `X-Webhook-Secret: <your-BETTERSTACK_WEBHOOK_SECRET>`

### 2. Configure the Webhook Template

Use this exact JSON template (Better Stack → Webhook → Template):

```json
{
  "event": "{{ incident.event }}",
  "data": {
    "id": "{{ incident.id }}",
    "type": "incident",
    "attributes": {
      "name": "{{ incident.name }}",
      "url": "{{ incident.url }}",
      "cause": "{{ incident.cause }}",
      "started_at": "{{ incident.started_at }}",
      "acknowledged_at": "{{ incident.acknowledged_at }}",
      "acknowledged_by": "{{ incident.acknowledged_by }}",
      "resolved_at": "{{ incident.resolved_at }}",
      "resolved_by": "{{ incident.resolved_by }}"
    }
  },
  "relationships": {
    "monitor": {
      "data": {
        "id": "{{ monitor.id }}",
        "type": "monitor"
      }
    }
  }
}
```

### 3. Get Your API Token

Better Stack → **Settings → API** → create a token with incident read/write permissions. Set it as `BETTERSTACK_API_TOKEN`.

## Telegram Setup

### 1. Create a Bot

1. Message [@BotFather](https://t.me/BotFather) on Telegram
2. Send `/newbot` and follow the prompts
3. Copy the token → set as `TELEGRAM_BOT_TOKEN`

### 2. Add the Bot to Your Group

1. Add the bot to your target Telegram group
2. Promote it to **admin** (needs permission to send and edit messages)

### 3. Get the Chat ID

Send a message in the group, then visit:
```
https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates
```
Look for `"chat":{"id":-100XXXXXXXXX}` — that negative number is your `TELEGRAM_CHAT_ID`.

## Deployment

### Requirements

- The service must be reachable over **HTTPS** from both Telegram and Better Stack
- `PUBLIC_URL` must be the externally reachable base URL (e.g. `https://bot.example.com`)
- The service registers its Telegram webhook on startup — no manual `setWebhook` needed

### Docker

```bash
# Pull the latest image
docker pull ghcr.io/ah-dark/betterstack-telegram-bot:latest

# Run with environment variables
docker run -d \
  --name betterstack-bot \
  --restart unless-stopped \
  -e TELEGRAM_BOT_TOKEN=... \
  -e TELEGRAM_CHAT_ID=... \
  -e PUBLIC_URL=https://bot.example.com \
  -e BETTERSTACK_WEBHOOK_SECRET=... \
  -e BETTERSTACK_API_TOKEN=... \
  -e REDIS_URL=redis://redis:6379/0 \
  -p 127.0.0.1:8080:8080 \
  ghcr.io/ah-dark/betterstack-telegram-bot:latest
```

### Health Check

```bash
curl http://localhost:8080/healthz
# → 200 OK
```

### Local Development

```bash
# Start Redis
docker run --rm -d -p 127.0.0.1:6379:6379 redis

# Run with pretty logging
LOG_FORMAT=pretty \
TELEGRAM_BOT_TOKEN=... \
TELEGRAM_CHAT_ID=... \
PUBLIC_URL=https://your-ngrok-url.ngrok.io \
BETTERSTACK_WEBHOOK_SECRET=dev-secret \
BETTERSTACK_API_TOKEN=... \
REDIS_URL=redis://127.0.0.1:6379/0 \
cargo run
```

Use [ngrok](https://ngrok.com) or [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/) to expose your local port for Telegram webhook registration.

## Horizontal Scaling

This service is designed for horizontal scaling:

- **Webhook mode** (not polling): Telegram delivers each update to exactly one replica via the load balancer. Long-polling would cause replicas to race for updates.
- **All state in Redis**: No in-process incident state. A replica restart or scale event loses nothing — the next webhook/callback rehydrates from Redis.
- **Idempotent webhooks**: Better Stack retries webhooks up to 10× with exponential backoff. The `SET NX EX` dedup key ensures duplicate deliveries are no-ops.
- **Startup webhook registration**: Every replica calls `setWebhook` with the same URL/secret on startup — idempotent, safe.

To scale horizontally, run multiple replicas behind a load balancer pointing to the same `PUBLIC_URL`, all connected to the same Redis instance.

## Release Process

This project uses [release-please](https://github.com/googleapis/release-please) for automated releases:

1. Merge conventional-commit PRs to `main` (e.g. `feat: ...`, `fix: ...`)
2. release-please maintains a release PR that bumps `Cargo.toml` version and `CHANGELOG.md`
3. Merging the release PR tags `v{X.Y.Z}` and creates a GitHub Release
4. The `docker-publish` workflow fires on release and pushes a multi-arch image to `ghcr.io/ah-dark/betterstack-telegram-bot`

## License

This project is licensed under the GNU Affero General Public License v3.0 — see the [LICENSE](LICENSE) file for details.
