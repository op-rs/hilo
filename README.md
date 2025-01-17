<h1 align="center">
    <img src="./assets/banner.png" alt="hilo" width="100%" align="center">
</h1>

<h4 align="center">
    A suite of `std` components for the superchain.
</h4>

<p align="center">
  <a href="https://github.com/op-rs/hilo/actions/workflows/rust_ci.yaml"><img src="https://github.com/op-rs/hilo/actions/workflows/rust_ci.yaml/badge.svg?label=ci" alt="CI"></a>
  <a href="https://github.com/op-rs/hilo/blob/main/LICENSE.md"><img src="https://img.shields.io/badge/License-MIT-d1d1f6.svg?label=license&labelColor=2a2f35" alt="License"></a>
  <a href="https://op-rs.github.io/hilo"><img src="https://img.shields.io/badge/Contributor%20Book-854a15?logo=mdBook&labelColor=2a2f35" alt="Book"></a>
  <a href="https://img.shields.io/codecov/c/github/op-rs/hilo"><img src="https://img.shields.io/codecov/c/github/op-rs/hilo" alt="Codecov"></a>
</p>

<p align="center">
  <a href="#overview">Overview</a> •
  <a href="#security">Security</a> •
  <a href="#contributing">Contributing</a> •
  <a href="#license">License</a>
</p>

## Overview

`hilo` is the sister of [`kona`][kona].
It is a suite of portable, modular `std` OP Stack components,
where `kona` contains `no_std` components.
`hilo` provides library components in [crates](./crates/), and
binary applications in [bin/](./bin/). Bins like `hera` compose
multiple crates from `hilo` and `kona`.

## Development Status

`hilo` is currently in active development, and is not yet ready for use in production.

## Security

Specifications around security are provided in [Security.md](./Security.md).

## Contributing

Contributing guidelines are outlined in [Contributing.md](./Contributing.md).

## License

Licensed under <a href="LICENSE-MIT">MIT license</a>.

<!-- Links -->

[mit-url]: LICENSE-MIT
[apache-url]: LICENSE-APACHE
[kona]: https://github.com/op-rs/kona
