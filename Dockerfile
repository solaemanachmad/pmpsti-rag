# ── Stage 1: Builder ──────────────────────────────────────────────
# ubuntu:24.04 ships glibc 2.39
FROM ubuntu:24.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y \
    curl \
    pkg-config \
    libssl-dev \
    cmake \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Install Rust 1.88 via rustup
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain 1.88.0 --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /app

# Cache dependencies first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Build the real binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────────────
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

# Only TLS certificates and OpenSSL are needed: pages are read as plain HTML (no headless
# browser — Chromium does not fit in the free Render instance's 512 MB).
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/pmpsti ./pmpsti

RUN chown -R ubuntu:ubuntu /app
USER ubuntu

EXPOSE 7860

ENV HOST=0.0.0.0
ENV PORT=7860

CMD ["./pmpsti"]
