# syntax=docker/dockerfile:1

# Stage 1: Chef - prepare recipe
FROM rust:1.98-slim-bookworm@sha256:ebd900bae66fd508b466cef82d64a83a5fb34682e4c8b2797a42908bddc95a57 AS chef
RUN cargo install cargo-chef --version 0.1.71 --locked
WORKDIR /app

# Stage 2: Planner - generate recipe.json
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder - build dependencies then binary
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies (cached layer)
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin betterstack-telegram-bot

# Stage 4: Runtime - distroless nonroot
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/betterstack-telegram-bot /app/betterstack-telegram-bot
EXPOSE 8080
ENTRYPOINT ["/app/betterstack-telegram-bot"]
