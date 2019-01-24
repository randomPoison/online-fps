# Online FPS

This is an experimental game using [Amethyst] and [SpatialOS].

## How To Run

1. Follow the [setup instructions for the SDK](https://github.com/jamiebrynes7/spatialos-sdk-rs#setup).
1. Install the `setup` and `generator` executables from the spatialos-sdk-rs repo:
    * `cargo install --path spatialos-sdk-tools --bin setup`
    * `cargo install --path spatialos-sdk-code-generator`
1. Run `setup schema`
    * Pass in the `--spatial-lib-dir` parameter explicitly if you don't have the
      `SPATIAL_LIB_DIR` environment variable set).
1. Run `generator schema\bin\bundle.json workers\server\src\generated.rs`
1. Run `spatial local launch --launch_config=deployment.json`
1. Run `cd client && cargo run`

[Amethyst]: https://amethyst-engine.org/
[SpatialOS]: https://improbable.io/games
