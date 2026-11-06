# Build stage runs natively on the build machine (e.g. an ARM Mac) and
# cross-compiles one static binary for the target platform (linux/amd64).
FROM --platform=$BUILDPLATFORM rust:1.99-slim AS build
ARG TARGETARCH
RUN case "$TARGETARCH" in arm64) echo aarch64 ;; *) echo x86_64 ;; esac > /arch \
 && rustup target add "$(cat /arch)-unknown-linux-musl"
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY static static
RUN target="$(cat /arch)-unknown-linux-musl" \
 && cargo build --release --locked --target "$target" \
      --config "target.$target.linker='rust-lld'" \
 && cp "target/$target/release/nas-dashboard" /nas-dashboard

# Final image: just the binary. No OS, no shell, nothing else to attack.
FROM scratch
LABEL org.opencontainers.image.source=https://github.com/BrunoMerenyi/omv_dash
COPY --from=build /nas-dashboard /nas-dashboard
USER 10001:10001
EXPOSE 6767
ENTRYPOINT ["/nas-dashboard"]
