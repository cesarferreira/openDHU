FROM rust:1.99.1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    ffmpeg \
    libxkbcommon-dev \
    libx11-dev \
    libxi-dev \
    libgl1-mesa-dev \
    protobuf-compiler \
    python3 \
    tesseract-ocr \
    tesseract-ocr-eng \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy rustfmt

ENV CARGO_TARGET_DIR=/app/target \
    CARGO_TERM_COLOR=never
WORKDIR /app
COPY . .

CMD ["sh", "-ec", "cargo fetch --locked && make all BIN=/app/target/debug && cargo test --locked -p opendhu-test --test real_ocr -- --ignored && cargo test --locked -p opendhu-native-hu screenshot::tests::a_long_gop_screenshot_keeps_the_final_blue_frame -- --ignored && make sample-validate OPENDHU_BIN=/app/target/debug/aa"]
