# Online FPS

This is an experimental game using [Amethyst] and [SpatialOS].

## How To Run

1. Follow the [setup instructions for the SDK](https://github.com/jamiebrynes7/spatialos-sdk-rs#setup).
2. Run `cargo run --bin setup -- schema` (pass in the `--spatial-lib-dir`
  parameter if you don't have the `SPATIAL_LIB_DIR` environment variable set).
3. Run `spatial local launch --launch_config=deployment.json`
4. Run `cd client && cargo run`

[Amethyst]: https://amethyst-engine.org/
[SpatialOS]: https://improbable.io/games
