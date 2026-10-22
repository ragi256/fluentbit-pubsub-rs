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
cargo build --locked --release
```

After building, the Linux shared library will be located at:

```text
target/release/libfluent_bit_pubsub_rs.so
```

To build an explicit Linux target:

```sh
cargo build --locked --release --target x86_64-unknown-linux-gnu
cargo build --locked --release --target aarch64-unknown-linux-gnu
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
  cargo build --locked --release --target x86_64-unknown-linux-gnu
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
VERSION=v0.1.1
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
  - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

  - name: Download plugin
    env:
      GH_TOKEN: ${{ github.token }}
      VERSION: v0.1.1
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
git tag -s v0.1.1
git push origin v0.1.1
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
| `Project` | Yes | — | Google Cloud Project ID. The plugin refuses to initialize when omitted. |
| `Topic` | Yes | — | Google Cloud Pub/Sub Topic Name. The plugin refuses to initialize when omitted. |
| `JwtPath` | No | — | Path to a `service_account` JSON key or `external_account` (Workload Identity Federation) configuration JSON. It is read directly; the plugin does not modify process environment variables. |
| `Jwt` | No | — | Raw `service_account` JSON key or `external_account` configuration JSON. This is intended for a FireLens secret injected as an environment expansion in the Fluent Bit configuration. |
| `MessageKey` | No | — | Send the named record value as the Pub/Sub message body. String and binary values are sent verbatim; other values are JSON encoded. A missing key is a non-retryable flush error. |
| `Attributes` | No | `{}` | Fixed Pub/Sub attributes for every message, as a JSON object with string values. They are separate from the message body. Invalid attributes fail initialization. |
| `Endpoint` | No | Google client default | Pub/Sub endpoint URL, for example `https://asia-northeast1-pubsub.googleapis.com`. Overrides emulator routing when both are configured. |
| `Timeout` | No | `60000` | Publish timeout in milliseconds. |
| `DelayThreshold` | No | `10` | Maximum batching delay in milliseconds. Must be a positive integer. |
| `ByteThreshold` | No | `1000000` | Batch byte threshold. Must be a positive integer. |
| `CountThreshold` | No | `100` | Batch message-count threshold. Must be a positive integer. |
| `Debug` | No | `false` | Enable enhanced debug logging (`true` / `false`). |

`Jwt` and `JwtPath` are mutually exclusive. When neither is specified, the
Google client library uses its normal Application Default Credentials behavior.

The batching defaults above are those of the pinned Google Pub/Sub client. Leaving
these settings unset delegates batching to that client; `Timeout` still limits
how long a Fluent Bit flush waits for the published messages.

### Optional attributes and endpoint

```ini
[OUTPUT]
    Name        pubsub
    Match       *
    Project     your-gcp-project-id
    Topic       your-pubsub-topic-name
    Attributes  {"service":"api","environment":"production"}
    Endpoint    https://asia-northeast1-pubsub.googleapis.com
```

Attributes can support subscription filtering without changing the JSON body or
its BigQuery columns. They are fixed per OUTPUT, and are not copied from record
fields. JSON supports commas and equals signs in attribute values without a
custom escaping syntax. At initialization the plugin enforces at most 100 entries,
non-empty keys up to 256 bytes that do not start with `goog`, and values up to
1024 bytes. These are UTF-8 byte lengths. Attribute bytes contribute to Pub/Sub
message size and quota usage. See [Pub/Sub attributes](https://docs.cloud.google.com/pubsub/docs/publisher#use_attributes_to_publish_a_message)
and [quotas and limits](https://docs.cloud.google.com/pubsub/quotas).

An explicit Endpoint changes the destination, but not authentication. Use HTTPS
for production; HTTP is accepted only when `PUBSUB_EMULATOR_HOST` is set. A URL
may include a port and a trailing slash, but not credentials, a path, a query,
or a fragment. If `PUBSUB_EMULATOR_HOST` is non-empty, the plugin uses anonymous
credentials, with Endpoint taking precedence over that variable for routing.
Keep `PUBSUB_EMULATOR_HOST` unset in production.

Leaving Endpoint unset preserves the Google client's normal endpoint selection
(or the existing emulator routing). Selecting a locational or regional endpoint
changes latency, quota locality, and regional-failure behavior, so choose a region
deliberately. A regional endpoint such as
`https://pubsub.asia-northeast1.rep.googleapis.com` also requires a compatible
message storage policy; it is not interchangeable with a locational endpoint.
See [Pub/Sub endpoints](https://docs.cloud.google.com/pubsub/docs/reference/service_apis_overview#pubsub_endpoints).
These options do not enable message ordering.

### FireLens migration from the Go plugin

By default, this plugin serializes the complete Fluent Bit record as the Pub/Sub
message body. This is the natural output-plugin behavior and preserves the Rust
plugin's existing behavior.

Some Go-plugin deployments wrap a JSON body in a record such as
`{"data":"{...}"}` before the output stage. To retain that pipeline unchanged,
set `MessageKey data`:

```ini
[OUTPUT]
    Name        pubsub
    Match       *
    Project     ${GCP_PROJECT}
    Topic       ${GCP_TOPIC}
    Jwt         ${SECURE_GCP_CREDENTIAL}
    MessageKey  data
```

That sends the contents of `data` (for example, `{...}`), rather than the outer
record. It is the appropriate configuration for BigQuery subscriptions whose
table schema describes the wrapped JSON fields rather than a `data` column.

For ECS on AWS, prefer a short-lived AWS Workload Identity Federation
`external_account` configuration passed through `JwtPath` (or `Jwt`) over a
long-lived service-account key. The AWS credentials or task role must still be
authorized by the WIF provider configuration.

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
5. Verifies the default routing and payload with no attributes
6. Recreates Fluent Bit with attributes and an explicit endpoint, then validates
   both payload and attributes. An unusable fallback host proves the explicit
   endpoint is used.

The Docker images are expected to run on the host Linux architecture, so this is
the preferred local smoke test for both x86_64 and arm64 machines.
The E2E test uses AWS's official Docker Hub mirror
(`amazon/aws-for-fluent-bit:3.4.4`) rather than an anonymous ECR Public pull,
while production can continue to use the equivalent Public ECR image.

GitHub Actions runs Linux checks on each PR:

- `.github/workflows/ci.yml`

CI/CD policy:

- `cargo fmt`, `cargo clippy`, and unit tests run on Linux.
- Release shared library builds use `rust:1.98-bookworm` for both
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. CI rejects an
  artifact that requires newer than `GLIBC_2.34`, matching
  `aws-for-fluent-bit:3.4.4`.
- Emulator E2E runs on both x86_64 and arm64 Linux runners.
- Release artifacts are packaged separately for x86_64 and arm64 Linux.

## License

This project is licensed under the [MIT License](LICENSE).
