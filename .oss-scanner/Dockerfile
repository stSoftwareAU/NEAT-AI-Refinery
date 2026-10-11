# OSS Scanner build image (https://github.com/anthropics/oss-scanner).
# The scanner builds this with the repository as the build context, then runs
# its audit offline, so every crate Refinery and its tests need is fetched
# here. The base image matches the channel pinned in rust-toolchain.toml.
FROM rust:1.99.0-trixie

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src

# Full debug info and frame pointers.
ENV CARGO_PROFILE_DEV_DEBUG=true \
    CARGO_PROFILE_TEST_DEBUG=true \
    RUSTFLAGS="-C force-frame-pointers=yes"

# Install the pinned toolchain's components, fetch every crate against the
# lockfile, then build the binary and every test, bench and example target.
RUN rustup toolchain install \
    && rustc --version \
    && cargo fetch --locked \
    && cargo build --locked --workspace --all-features \
    && cargo test --locked --workspace --all-features --all-targets --no-run

# From here on the audit runs offline; keep cargo from reaching for the network.
ENV CARGO_NET_OFFLINE=true

# Smoke-run the unit tests and the transform tests; a failure warns rather than
# aborting the build. The full suite is
# `cargo test --workspace --all-features -- --test-threads=2`.
RUN (timeout 900 cargo test --locked -p neat-ai-refinery --all-features --lib \
      --test record_stream --test transform_publish --test sample_transform \
      --test quantise_transform --test fuzz_transform --test pipeline_compose \
      --test exit_codes -- --test-threads=2 \
      || echo "WARNING: smoke tests reported failures")
