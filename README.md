# Tooling for OpenWRT nodes

Various Rust-based

## Toolchain

- Build the toolchain: `docker build . --target buildroot -t openwrt-builder`
  - Can be skipped once we have pushed images
- Build the project for the platform:

  ```
  docker run -it --rm -v $PWD:/src -w /src openwrt-builder cargo build
  ```

  - add `--release` to optimize for size
