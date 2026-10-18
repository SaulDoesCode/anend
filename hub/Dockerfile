# The anend server as a container. Build from the repository's root:
#
#   docker build -f hub/Dockerfile -t anend-hub .
#   docker run -d -p 8080:8080 -v anend-data:/data anend-hub --url https://hub.example.org
#
# Everything after the image name goes to anend-hub (see `anend-hub --help`): add
# `--mode app` for the web app alone, `--mode hub` for the sharing site alone, and
# `--admin YOURNAME` once to print the first admin's login link (`docker logs`).
# For a build that leaves a half out: --build-arg FEATURES="--no-default-features --features app"

FROM rust:1-bookworm AS build
ARG FEATURES=""
WORKDIR /src
COPY . .
# the web app (app/, WebAssembly) goes into the hub's binary
RUN rustup target add wasm32-unknown-unknown \
    && cargo install --locked wasm-bindgen-cli --version 0.2.129 \
    && sh app/web/build.sh
RUN cargo build --release --locked -p anend-hub $FEATURES

FROM debian:bookworm-slim
RUN useradd --system --home /data --shell /usr/sbin/nologin anend \
    && install -d -o anend -g anend /data
COPY --from=build /src/target/release/anend-hub /usr/local/bin/anend-hub
USER anend
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD ["/bin/sh", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /health HTTP/1.0\\r\\nHost: 127.0.0.1\\r\\n\\r\\n' >&3 && grep -q '\"ok\"' <&3"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["anend-hub", "--data", "/data", "--addr", "0.0.0.0:8080"]
