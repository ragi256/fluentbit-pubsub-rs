# fluent-bit-pubsub-rs

A [Fluent Bit](https://fluentbit.io/) output plugin for Google Cloud Pub/Sub written in Rust.

This plugin allows Fluent Bit to send log records directly to Google Cloud Pub/Sub topics.

## Requirements

- **Rust** (Edition 2024) and **Cargo**
- **Fluent Bit** (v1.9 or higher is recommended, compiled with dynamic plugin support)

## Build

To build the plugin, run the following command in the project root:

```sh
cargo build --release
```

After building, the shared library will be located in the `target/release/` directory:
- Linux: `target/release/libfluent_bit_pubsub_rs.so`
- macOS: `target/release/libfluent_bit_pubsub_rs.dylib`
- Windows: `target/release/fluent_bit_pubsub_rs.dll`

## Usage Configuration

Load the compiled plugin into Fluent Bit and configure the `[OUTPUT]` section as follows:

```ini
[SERVICE]
    Plugins_File plugins.conf

[INPUT]
    Name dummy
    Tag  test.log

[OUTPUT]
    Name      pubsub
    Match     *
    Project   your-gcp-project-id
    Topic     your-pubsub-topic-name
    # Optional parameters
    JwtPath   /path/to/your/gcp-credentials.json
    Timeout   60000
    Debug     false
```

### Configuration Parameters

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| `Project` | Yes | `default-project` | Google Cloud Project ID. |
| `Topic` | Yes | `default-topic` | Google Cloud Pub/Sub Topic Name. |
| `JwtPath` | No | | Path to the GCP Service Account JSON key file. Sets `GOOGLE_APPLICATION_CREDENTIALS` internally. |
| `Jwt` | No | | GCP Service Account credentials as a raw JSON string. If provided, sets `GOOGLE_APPLICATION_CREDENTIALS_JSON` internally. (Use `JwtPath` preferably) |
| `Timeout` | No | `60000` | Publish timeout in milliseconds. |
| `Debug` | No | `false` | Enable enhanced debug logging (`true` / `false`). |

## Emulator E2E (Local and CI)

This repository includes an end-to-end test that runs Fluent Bit with this plugin
against the Google Cloud Pub/Sub emulator.

- Local run:

```sh
bash e2e/e2e_emulator.sh
```

What this script does:

1. Builds the Linux shared library in Docker (`libfluent_bit_pubsub_rs.so`)
2. Starts the Pub/Sub emulator
3. Creates a topic/subscription in the emulator
4. Runs Fluent Bit with dummy input and this plugin
5. Pulls a message from emulator subscription and validates payload

GitHub Actions runs the same script on each PR:

- `.github/workflows/ci.yml`

## License

This project is licensed under the [MIT License](LICENSE).
