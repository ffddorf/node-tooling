# useful for integration tests, not cross-compiling
ARG PLATFORM=aarch64_generic
FROM --platform=linux/${PLATFORM} openwrt/rootfs:${PLATFORM}-24.10.6 AS rootfs

RUN mkdir -p /var/lock && \
  opkg update && \
  opkg install curl gcc

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH

# --- toolchain docker targets starting here

FROM ubuntu:24.04 AS toolchain

RUN apt-get update && \
  apt-get -y --no-install-recommends install \
  build-essential clang flex bison g++ gawk \
  gettext git libncurses5-dev libssl-dev \
  python3-setuptools rsync swig unzip zlib1g-dev file wget curl ca-certificates \
  # todo: customize by arch
  gcc-multilib-mipsel-linux-gnu g++-multilib-mipsel-linux-gnu

RUN useradd -ms /bin/bash builder && \
  mkdir -p /src && chown -R builder /src
USER builder

RUN git clone https://github.com/openwrt/openwrt.git /src
WORKDIR /src

# todo: limit feeds to base & packages

RUN ./scripts/feeds update -a && \
  ./scripts/feeds install -a

RUN cat > .config <<EOF
CONFIG_TARGET_ramips=y
CONFIG_TARGET_ramips_mt7621=y
CONFIG_TARGET_ramips_mt7621_DEVICE_genexis_pulse-ex400=y
CONFIG_TARGET_BOARD="ramips"
CONFIG_TARGET_ARCH_PACKAGES="mipsel_24kc"
CONFIG_mipsel=y
CONFIG_ARCH="mipsel"
EOF
RUN make defconfig

ENV MAKEFLAGS="-j10"
RUN make toolchain/install
RUN make package/rust/host/compile
RUN make package/system/uci/compile

FROM ubuntu:24.04 AS buildroot

RUN apt-get update && \
  apt-get -yq install \
  curl ca-certificates \
  build-essential clang g++ \
  gcc-multilib-mipsel-linux-gnu g++-multilib-mipsel-linux-gnu

COPY --from=toolchain /src/staging_dir /staging_dir

ENV PATH=/staging_dir/host/bin:$PATH
ENV PATH=/staging_dir/toolchain-mipsel_24kc_gcc-14.3.0_musl/bin:$PATH
ENV PATH=/staging_dir/target-mipsel_24kc_musl/host/bin:$PATH
ENV UCI_DIR=/staging_dir/target-mipsel_24kc_musl/usr

ENV CC=gcc
ENV CARGO_BUILD_TARGET=mipsel-unknown-linux-musl
ENV CARGO_TARGET_MIPSEL_UNKNOWN_LINUX_MUSL_LINKER=mipsel-openwrt-linux-gcc
