# ---- Builder ----
	FROM rust:1-bookworm AS builder
	WORKDIR /app
	
	RUN apt-get update && apt-get install -y --no-install-recommends \
		pkg-config libssl-dev \
		&& rm -rf /var/lib/apt/lists/*
	
	COPY Cargo.toml Cargo.lock ./
	COPY .sqlx ./.sqlx
	COPY migrations ./migrations
	COPY src ./src
	
	ENV SQLX_OFFLINE=true
	RUN cargo build --release
	
	# ---- Runtime ----
	FROM debian:bookworm-slim AS runtime
	WORKDIR /app
	
	RUN apt-get update && apt-get install -y --no-install-recommends \
		ca-certificates libssl3 \
		&& rm -rf /var/lib/apt/lists/*
	
	COPY --from=builder /app/target/release/bridge ./bridge
	
	EXPOSE 8080
	ENTRYPOINT ["./bridge"]