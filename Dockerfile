# Stage 1: Builder
FROM rust:1.77-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release && strip /app/target/release/ember

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r ember && useradd -r -g ember -d /app -s /sbin/nologin ember

WORKDIR /app

COPY --from=builder /app/target/release/ember /usr/local/bin/ember

RUN chown -R ember:ember /app

USER ember

EXPOSE 443 9090

ENTRYPOINT ["ember"]
