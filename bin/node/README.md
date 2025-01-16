## `node`

A rollup or consensus node powered by [hilo] and [kona].

It can be run as either a standalone node or as an [Execution Extension][exex]
on top of an L1 [Reth][reth] node in the same process.

Under the hood, the node is powered by the [Kona-derive][kona] library which handles
the [derivation pipeline][derivation] of the L2 payloads from L1 transactions.

### Usage

```
cargo run --bin node
```

<!-- Links -->

[hilo]: https://github.com/op-rs/hilo
[kona]: https://github.com/op-rs/kona
[reth]: https://github.com/paradigmxyz/reth
[exex]: https://www.paradigm.xyz/2024/05/reth-exex
[opstack]: https://docs.optimism.io/
[derivation]: https://docs.optimism.io/stack/protocol/derivation-pipeline
