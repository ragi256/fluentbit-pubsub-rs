# fluent-bit-pubsub-rs

A [Fluent Bit](https://fluentbit.io/) output plugin for Google Cloud Pub/Sub written in Rust.

This plugin allows Fluent Bit to send log records directly to Google Cloud Pub/Sub topics.

## Requirements

- **Rust** (Edition 2024) and **Cargo**
- **Fluent Bit** (v1.9 or higher is recommended, compiled with dynamic plugin support)
- **Linux** on `x86_64` or `arm64`

## Target Environment

This repository targets Linux only. The supported production, development, and
CI/CD targets are:

| CPU architecture | Rust target |
|------------------|-------------|
| x86_64 | `x86_64-unknown-linux-gnu` |
| arm64 | `aarch64-unknown-linux-gnu` |

Non-Linux operating systems are intentionally out of scope. Use Docker for local
checks when the host machine is not the production-like environment.

## Build

To build the plugin, run the following command in the project root:

```sh
cargo build --release
```

After building, the Linux shared library will be located at:

```text
target/release/libfluent_bit_pubsub_rs.so
```

To build an explicit Linux target:

```sh
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

Cross-building for `aarch64-unknown-linux-gnu` requires the Rust target and an
aarch64 Linux linker:

```sh
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install gcc-aarch64-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

When cross-building `x86_64-unknown-linux-gnu` from an arm64 Linux host, install
an x86_64 Linux linker and pass it to Cargo:

```sh
rustup target add x86_64-unknown-linux-gnu
sudo apt-get install gcc-x86-64-linux-gnu
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
  cargo build --release --target x86_64-unknown-linux-gnu
```

## Release Artifacts

GitHub Releases publish prebuilt Linux shared libraries for each supported Rust target:

- `fluent-bit-pubsub-rs-<version>-x86_64-unknown-linux-gnu.so`
- `fluent-bit-pubsub-rs-<version>-aarch64-unknown-linux-gnu.so`

Each `.so` is published with a matching `.sha256` file and a GitHub Artifact Attestation.
The checksum confirms that the downloaded file matches the release asset. The attestation
confirms that the `.so` was produced by this repository's release workflow.

Example verification:

```sh
VERSION=v0.1.0
TARGET=x86_64-unknown-linux-gnu
ASSET="fluent-bit-pubsub-rs-${VERSION}-${TARGET}.so"

gh release download "${VERSION}" \
  --repo ragi256/fluentbit-pubsub-rs \
  --pattern "${ASSET}" \
  --pattern "${ASSET}.sha256"

sha256sum -c "${ASSET}.sha256"

gh attestation verify "${ASSET}" \
  --repo ragi256/fluentbit-pubsub-rs \
  --signer-workflow ragi256/fluentbit-pubsub-rs/.github/workflows/release.yml@refs/tags/${VERSION}
```

When using the plugin in a Docker image build, download and verify it before `docker build`:

```yaml
steps:
  - uses: actions/checkout@v7

  - name: Download plugin
    env:
      GH_TOKEN: ${{ github.token }}
      VERSION: v0.1.0
      TARGET: x86_64-unknown-linux-gnu
    run: |
      asset="fluent-bit-pubsub-rs-${VERSION}-${TARGET}.so"
      gh release download "${VERSION}" \
        --repo ragi256/fluentbit-pubsub-rs \
        --pattern "${asset}" \
        --pattern "${asset}.sha256"
      sha256sum -c "${asset}.sha256"
      gh attestation verify "${asset}" \
        --repo ragi256/fluentbit-pubsub-rs \
        --signer-workflow "ragi256/fluentbit-pubsub-rs/.github/workflows/release.yml@refs/tags/${VERSION}"

  - name: Build image
    run: docker build -t my-fluent-bit-image .
```

To publish a release, create and push a signed tag:

```sh
git tag -s v0.1.0
git push origin v0.1.0
```

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

1. Builds the native Linux shared library in Docker (`libfluent_bit_pubsub_rs.so`)
2. Starts the Pub/Sub emulator
3. Creates a topic/subscription in the emulator
4. Runs Fluent Bit with dummy input and this plugin
5. Pulls a message from emulator subscription and validates payload

The Docker images are expected to run on the host Linux architecture, so this is
the preferred local smoke test for both x86_64 and arm64 machines.

GitHub Actions runs Linux checks on each PR:

- `.github/workflows/ci.yml`

CI/CD policy:

- `cargo fmt`, `cargo clippy`, and unit tests run on Linux.
- Release shared library builds are checked for both `x86_64-unknown-linux-gnu`
  and `aarch64-unknown-linux-gnu`.
- Emulator E2E runs on both x86_64 and arm64 Linux runners.
- Release artifacts are packaged separately for x86_64 and arm64 Linux.

## License

This project is licensed under the [MIT License](LICENSE).
