# Build image for local CI: same Ubuntu release and packages as the GitHub
# workflow, plus Node 22 and stable Rust.
FROM node:22-bookworm-slim AS node

FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl wget file git pkg-config rsync xdg-utils \
        libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev \
        libssl-dev libdbus-1-dev libxdo-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable

# The AppImage tools are AppImages themselves; containers have no FUSE, so
# tell them to unpack and run instead.
ENV APPIMAGE_EXTRACT_AND_RUN=1 \
    CARGO_TARGET_DIR=/cache/target \
    npm_config_cache=/cache/npm \
    CI=true

COPY run.sh /usr/local/bin/ci-run
RUN sed -i 's/\r$//' /usr/local/bin/ci-run && chmod +x /usr/local/bin/ci-run

WORKDIR /work
ENTRYPOINT ["ci-run"]
