# forgeos

ForgeOS CLI — a single static Rust binary that talks to the ForgeOS platform API.

## Build

```bash
cargo build --release
# binary at target/release/forgeos
```

## Usage

```bash
forgeos health                       # platform health
forgeos deploy agent.yaml            # validate + deploy a manifest
forgeos list [--json]                # list agents
forgeos describe <agent_id>          # full manifest + status detail
forgeos invoke <agent_id> "prompt"   # fire-and-return; --wait to block on the result
forgeos logs <agent_id> [--follow]   # merged run + tool-call activity stream
forgeos undeploy <agent_id>          # remove an agent
```

The server URL and token are read from `~/.forgeos/server.lock`, or `--remote` /
`--token` flags (also `FORGEOS_REMOTE` / `FORGEOS_TOKEN` env vars).

## License

BUSL-1.1
