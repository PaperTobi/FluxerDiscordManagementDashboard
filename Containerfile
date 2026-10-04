# Profanity Watch (the Rust bot), x86_64 or aarch64.
#
#   podman build -t profanity-watch .
#
# Stages: build (the program, the web UI's browser bundle, espeak-ng), weights (the pinned models and voices, fetched by
# the program itself and checked against their SHA-256; kept in a build cache so a code change does not download them
# again), and a small runtime image (glibc: LiveKit's libwebrtc needs it). Nothing is downloaded when the bot runs.

# ---------------------------------------------------------------------------------------------------------- build
FROM docker.io/library/rust:1.99-bookworm AS build

# apt drops to this user to download. Builders without user namespaces (chroot isolation) cannot: pass
# --build-arg APT_SANDBOX_USER=root there.
ARG APT_SANDBOX_USER=_apt
# clang 21 from LLVM's own repository (the libc++ inside LiveKit's prebuilt libwebrtc needs clang >= 21; bookworm ships
# 14), glib headers (libwebrtc), cmake and ninja (espeak-ng)
RUN apt-get -o APT::Sandbox::User=$APT_SANDBOX_USER update \
 && apt-get -o APT::Sandbox::User=$APT_SANDBOX_USER install -y --no-install-recommends ca-certificates curl gnupg \
 && install -d /etc/apt/keyrings \
 && curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key | gpg --dearmor -o /etc/apt/keyrings/llvm.gpg \
 && echo "deb [signed-by=/etc/apt/keyrings/llvm.gpg] http://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-21 main" \
      > /etc/apt/sources.list.d/llvm.list \
 && apt-get -o APT::Sandbox::User=$APT_SANDBOX_USER update \
 && apt-get -o APT::Sandbox::User=$APT_SANDBOX_USER install -y --no-install-recommends \
      clang-21 lld-21 libglib2.0-dev pkg-config cmake ninja-build git \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown
ENV CC=clang-21 CXX=clang++-21

WORKDIR /src
COPY . .
# The registry and target directories are build caches: a rebuild after a code change only compiles what changed.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo xtask espeak-ng \
 && cargo xtask web \
 && cargo build --release --locked -p pb \
 && install -D target/release/pb /out/bin/pb \
 && cp -r target/site /out/site \
 && install -d /out/espeak \
 && cp -r target/espeak-ng/share/espeak-ng-data /out/espeak/

# ---------------------------------------------------------------------------------------------------------- weights
FROM build AS weights
RUN --mount=type=cache,target=/cache/weights \
    /out/bin/pb fetch-weights --dest /cache/weights \
 && /out/bin/pb fetch-weights --dest /cache/weights --check \
 && cp -r /cache/weights /out/weights

# ---------------------------------------------------------------------------------------------------------- runtime
FROM docker.io/library/debian:bookworm-slim
ARG APT_SANDBOX_USER=_apt

LABEL org.opencontainers.image.title="profanity-watch" \
      org.opencontainers.image.description="Follows chosen Fluxer users into voice, scores what they say with the Roblox voice-safety model and warns them" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later; models: Roblox model licence (classifier), MIT (Silero VAD), voices: see /opt/pb/weights"

# ca-certificates: the system's trusted roots for TLS (Mozilla's are built in as well)
RUN apt-get -o APT::Sandbox::User=$APT_SANDBOX_USER update \
 && apt-get -o APT::Sandbox::User=$APT_SANDBOX_USER install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 10001 pb \
 && useradd --system --uid 10001 --gid 10001 --home-dir /data --no-create-home --shell /usr/sbin/nologin pb \
 && install -d -o 10001 -g 10001 -m 0750 /data

COPY --from=build /out/bin/pb /opt/pb/bin/pb
COPY --from=build /out/site /opt/pb/site
COPY --from=build /out/espeak /opt/pb/espeak
COPY --from=weights /out/weights /opt/pb/weights
COPY clips/ /opt/pb/clips/

# /data holds everything the bot keeps (settings, secrets entered in the web UI, the event log, recordings, uploads):
# mount a volume there. The web UI (setup wizard, management, /healthz) listens on 8790.
ENV PB_DATA=/data PATH=/opt/pb/bin:$PATH
USER 10001:10001
WORKDIR /data
VOLUME /data
EXPOSE 8790
STOPSIGNAL SIGTERM
# Health (OCI images drop HEALTHCHECK; use it from the host or a quadlet): pb health
ENTRYPOINT ["/opt/pb/bin/pb"]
CMD ["run"]
