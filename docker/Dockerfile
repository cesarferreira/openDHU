# Headless Android emulator with `aa` for running openDHU suites on a Linux host with KVM.
#   docker build -f docker/Dockerfile -t opendhu .
# See docs/docker.md.
ARG UBUNTU=24.04

FROM ubuntu:${UBUNTU} AS build
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
COPY . /src
WORKDIR /src
RUN cargo build --release --locked -p opendhu-cli -p opendhu-native-hu

FROM ubuntu:${UBUNTU}
ARG ANDROID_API=36
ARG CMDLINE_TOOLS=11076708
# The runtime user has no passwd entry, so Java would resolve user.home as "?" and miss the debug keystore.
ENV DEBIAN_FRONTEND=noninteractive \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    ANDROID_AVD_HOME=/avd \
    ANDROID_API=${ANDROID_API} \
    HOME=/home/opendhu \
    JAVA_TOOL_OPTIONS=-Duser.home=/home/opendhu
ENV PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/opt/android-sdk/emulator:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl unzip openjdk-17-jdk-headless ffmpeg tesseract-ocr tesseract-ocr-eng \
        libssl3 lsof procps iproute2 \
        libpulse0 libnss3 libx11-6 libxcb1 libgl1 libasound2t64 libxkbcommon0 libdbus-1-3 \
        libxcomposite1 libxcursor1 libxdamage1 libxfixes3 libxi6 libxrandr2 libxtst6 \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p ${ANDROID_HOME}/cmdline-tools \
    && curl -fsSL "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_TOOLS}_latest.zip" -o /tmp/tools.zip \
    && unzip -q /tmp/tools.zip -d ${ANDROID_HOME}/cmdline-tools \
    && mv ${ANDROID_HOME}/cmdline-tools/cmdline-tools ${ANDROID_HOME}/cmdline-tools/latest \
    && rm /tmp/tools.zip \
    && (yes | sdkmanager --licenses >/dev/null) \
    && sdkmanager "platform-tools" "emulator" "system-images;android-${ANDROID_API};google_apis_playstore;x86_64" >/dev/null

COPY --from=build /src/target/release/aa /src/target/release/opendhu-native-hu /usr/local/bin/
COPY docker/entrypoint.sh /usr/local/bin/opendhu-entrypoint

# The runtime user is chosen with `docker run --user`, so the writable locations stay open.
RUN mkdir -p /home/opendhu /avd \
    && chmod +x /usr/local/bin/opendhu-entrypoint \
    && chmod -R a+rwX /home/opendhu /avd /opt/android-sdk

WORKDIR /work
ENTRYPOINT ["opendhu-entrypoint"]
CMD ["test"]
