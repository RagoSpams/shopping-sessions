FROM rust:1.90.0-slim-bookworm@sha256:64232e656c058f4468e8d024e990acff04f0fd5a5c0a88a574dc37773d7325c9 AS build
WORKDIR /build
COPY rust/Cargo.toml rust/Cargo.lock ./
COPY rust/src ./src
RUN RUSTFLAGS="-C target-feature=+crt-static -C target-cpu=x86-64-v3" \
    cargo build --release --locked --features redesign --bin shopping-redesign --target x86_64-unknown-linux-gnu

FROM scratch
COPY --from=build /build/target/x86_64-unknown-linux-gnu/release/shopping-redesign /shopping-redesign
USER 1000:1000
CMD ["/shopping-redesign"]
