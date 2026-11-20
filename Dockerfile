# syntax=docker/dockerfile:1@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183

# Stage 1: Chef - prepare recipe
FROM rust:1.96-slim-bookworm@sha256:e18a79fc84dfcfc3ab5ba72290398a644c135c97eaa881447fddc354ee4701a3 AS chef
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
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/betterstack-telegram-bot /app/betterstack-telegram-bot
EXPOSE 8080
ENTRYPOINT ["/app/betterstack-telegram-bot"]
