# Jito BAM direct bundle examples

Runnable Rust and Python examples for submitting an ordered, atomic bundle of
one to five fully signed Solana transactions directly to a Jito BAM node.

Rust 1.88 or newer is required for the Rust clients. Python 3.9 or newer is
required for the Python clients.

> [!IMPORTANT]
> These endpoints are for testing. Review the [Jito Terms of Use] and obtain
> the appropriate credentials from Jito before sending traffic.

| Transport          | Endpoint                                                         | Credential                           | Success means                                                                      |
|--------------------|------------------------------------------------------------------|--------------------------------------|------------------------------------------------------------------------------------|
| JSON-RPC over HTTP | `http://<region>.mainnet.bam.jito.wtf:9090/api/v1/bundles`       | UUID in `x-jito-auth`                | BAM accepted the bundle into its ingress queue and returned a bundle ID.           |
| QUIC over UDP      | `<region>.mainnet.bam.jito.wtf:11228`                            | Allowlisted Ed25519 client keypair   | The peer was reachable after the frame write. No delivery result or bundle ID is returned. |

The credentials are separate and have separate rate limits. Use a dedicated
QUIC authentication keypair; do not reuse a trading wallet or transaction fee
payer. Transactions still need all normal Solana signatures.

Choose active regions with the [BAM node explorer]. The regional
`http://<region>.mainnet.bam.jito.wtf:9090/api/v1/validators` endpoint reports
connected leaders for one node in real time. Send to every active region when
integrating across the network. New or changed credentials can take up to 80
seconds to reach every BAM node.

## Rust examples

Build both Rust clients:

```bash
cargo build --release
```

Submit with JSON-RPC over HTTP:

```bash
export BAM_BUNDLE_URL='http://ewr.mainnet.bam.jito.wtf:9090/api/v1/bundles'
export JITO_AUTH_UUID='<provided-uuid>'
export SIGNED_TX_B64='<base64-signed-transaction>'

./target/release/send-bam-json-rpc-bundle "$SIGNED_TX_B64"
```

Submit the same transaction as a BAMB v0 frame over authenticated QUIC:

```bash
export BAM_QUIC_ADDR='ewr.mainnet.bam.jito.wtf:11228'
export BAM_QUIC_KEYPAIR='/path/to/allowlisted-quic-auth-keypair.json'

./target/release/send-bam-quic-bundle "$SIGNED_TX_B64"
```

Pass up to five transaction arguments to either binary. Their order is the
bundle order. The Rust JSON-RPC client is in
[`src/bin/send_bam_json_rpc_bundle.rs`](src/bin/send_bam_json_rpc_bundle.rs).
The authenticated QUIC client is in
[`src/bin/send_bam_quic_bundle.rs`](src/bin/send_bam_quic_bundle.rs), and its
shared base64 validation and BAMB encoder are in [`src/lib.rs`](src/lib.rs).

## Python JSON-RPC example

The JSON-RPC client uses only the Python standard library. It accepts base64
only. There is no base58 branch, and every request includes the required
`{"encoding":"base64"}` configuration.

```bash
export BAM_BUNDLE_URL='http://ewr.mainnet.bam.jito.wtf:9090/api/v1/bundles'
export JITO_AUTH_UUID='<provided-uuid>'
export SIGNED_TX_B64='<base64-signed-transaction>'

python3 send_bam_json_rpc_bundle.py "$SIGNED_TX_B64"
```

Example output:

```text
bundle_id=<bundle-id>
accepted_by_ingress=true (this does not guarantee the bundle will land)
```

Pass up to five transactions to preserve their bundle order:

```bash
export SIGNED_TX_1_B64='<base64-signed-transaction-1>'
export SIGNED_TX_2_B64='<base64-signed-transaction-2>'

python3 send_bam_json_rpc_bundle.py "$SIGNED_TX_1_B64" "$SIGNED_TX_2_B64"
```

The UUID is read only from `JITO_AUTH_UUID`, which keeps it out of command
history and process arguments. The endpoint currently supports HTTP, not
HTTPS, so send the credential only to the BAM host Jito provided.

Common HTTP outcomes:

| Status                    | Meaning                                                     |
|---------------------------|-------------------------------------------------------------|
| `401 Unauthorized`        | The UUID is missing or unknown.                             |
| `429 Too Many Requests`   | The credential exceeded its bundle-submission limit.        |
| `503 Service Unavailable` | The bundle access policy is temporarily unavailable.        |

## Python QUIC example

Create an environment and install the two direct dependencies:

```bash
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install -r requirements.txt
```

Then submit a bundle:

```bash
export BAM_QUIC_ADDR='ewr.mainnet.bam.jito.wtf:11228'
export BAM_QUIC_KEYPAIR='/path/to/allowlisted-quic-auth-keypair.json'
export SIGNED_TX_B64='<base64-signed-transaction>'

python3 send_bam_quic_bundle.py "$SIGNED_TX_B64"
```

The script derives a temporary self-signed client certificate from the 64-byte
Solana keypair file, negotiates ALPN `solana-tpu`, and writes one BAMB v0 frame
to one unidirectional QUIC stream. Temporary certificate files are removed
automatically.

Example output:

```text
transaction_count=1
transaction_1_bytes=284
bamb_frame_bytes=310
quic_ping=ack (peer reachable after stream write)
quic_send=ok (write completed; delivery and landing are unconfirmed)
```

### BAMB v0 frame

All integers are little-endian. The fixed 26-byte header is followed by the raw
signed transaction bytes in bundle order.

| Offset | Size     | Field                   | Required value                                    |
|-------:|---------:|-------------------------|---------------------------------------------------|
| 0      | 4        | Magic                   | ASCII `BAMB`                                      |
| 4      | 1        | Version                 | `0`                                               |
| 5      | 1        | Flags                   | `0`                                               |
| 6      | 1        | Transaction count       | `1` through `5`                                   |
| 7      | 1        | Header length           | `26`                                              |
| 8      | 8        | Maximum scheduling slot | `0`; expiry is not supported                      |
| 16     | 10       | Transaction lengths     | Five `u16` values; unused entries are `0`         |
| 26     | variable | Payloads                | Concatenated raw signed transactions              |

These examples keep each serialized transaction within the standard
1,232-byte Solana packet size.

The Rust and Python QUIC clients implement this same frame layout.

Common QUIC outcomes:

| Outcome                                    | Likely meaning                                                                                                        |
|--------------------------------------------|-----------------------------------------------------------------------------------------------------------------------|
| Handshake failure or immediate close       | The credential is unknown or not yet propagated, or UDP is unreachable.                                              |
| Successful stream write but no landing     | BAM may have rate-limited, rejected, or not selected the bundle, or it arrived outside a useful leader window.        |
| Only some test transactions appear in RPC  | They may be from different bundles; one BAMB v0 bundle should not land partially.                                     |

## Confirm landing

Neither acknowledgement guarantees selection or landing. Check each submitted
transaction signature through a normal Solana RPC endpoint:

```bash
export SOLANA_RPC_URL='https://api.mainnet-beta.solana.com'
export TX_SIGNATURE='<submitted-transaction-signature>'

curl -sS "$SOLANA_RPC_URL" \
  --header 'content-type: application/json' \
  --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getSignatureStatuses\",\"params\":[[\"$TX_SIGNATURE\"],{\"searchTransactionHistory\":true}]}"
```

Bundles execute atomically and in order. If one transaction fails, the bundle
does not land partially. Bundle expiry, partial execution, base58 JSON-RPC
input, a bundle-status method, and an application-level QUIC response are not
supported by these direct endpoints.

The clients send exactly one bundle and exit. They do not create or sign
transactions, and they spend SOL only through the signed transactions you
provide.

Start functional testing at one bundle per second per node. The initial
documented limit is 25 bundles per second per credential per BAM node unless
Jito approves a different limit.

## Test

The tests validate the JSON-RPC request and authentication header, reject
invalid input before network I/O, and pin the BAMB v0 frame layout:

```bash
cargo test --all-targets
python3 -m unittest -v
```

## References

- JSON-RPC request shape adapted from Jito Labs' Apache-2.0-licensed
  [`json-rpc-client`], reduced to the `sendBundle` path and base64 encoding.
- QUIC framing, client flow, access setup, and operational guidance adapted
  from the Jito BAM Direct Bundle Ingest testing documentation.

## License

Apache-2.0. See [`LICENSE`](LICENSE).

[BAM node explorer]: https://explorer.bam.dev/api/v1/validators
[Jito Terms of Use]: https://www.jito.wtf/terms-of-use/
[`json-rpc-client`]: https://github.com/jito-labs/json-rpc-client
