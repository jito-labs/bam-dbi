# BAM direct bundle examples

Small clients for submitting bundles directly to BAM. Each transport is an
independent Cargo workspace crate so its dependencies stay isolated. Both
accept one to five signed wire-format transactions in bundle order.

## Signed transfer quickstart

The two `send_transfer` examples fetch a fresh blockhash, build and sign an
Agave V0 system transfer, serialize it with `wincode`, and submit it as a
one-transaction bundle. Use a funded payer and a recipient appropriate for the
cluster selected by `SOLANA_RPC_URL`:

```bash
export SOLANA_RPC_URL='<solana-rpc-url>'
export PAYER_KEYPAIR='/path/to/funded-payer.json'
export RECIPIENT='<recipient-pubkey>'
export LAMPORTS=1
```

## JSON-RPC

Set the BAM HTTP endpoint and credential, then run the complete transfer
example:

```bash
export BAM_BUNDLE_URL='<provided-bam-url>/api/v1/bundles'
export JITO_AUTH_UUID='<provided-uuid>'

cargo run --release -p bam-json-rpc --example send_transfer
```

To submit transactions already prepared by another process, pass each one as a
base64-encoded command-line argument:

```bash
export SIGNED_TX_B64='<base64-signed-transaction>'

cargo run --release -p bam-json-rpc -- "$SIGNED_TX_B64"
```

A returned bundle ID means BAM accepted the bundle into its ingress queue. It
does not guarantee the bundle will land.

## QUIC

Set the BAMB QUIC endpoint and allowlisted credential, then run the same
transfer over QUIC. `BAM_QUIC_KEYPAIR` authenticates the connection;
`PAYER_KEYPAIR` signs the transaction.

```bash
export BAM_QUIC_ADDR='<provided-host>:<port>'
export BAM_QUIC_KEYPAIR='/path/to/allowlisted-keypair.json'

cargo run --release -p bam-quic --example send_transfer
```

The QUIC binary also accepts prepared base64 transactions:

```bash
export SIGNED_TX_B64='<base64-signed-transaction>'

cargo run --release -p bam-quic -- "$SIGNED_TX_B64"
```

A successful send means the peer acknowledged the stream bytes. QUIC returns
no bundle ID or application response, so acceptance and landing are
unconfirmed.

The tests cover legacy, V0, and BAM V1 transactions, request limits, JSON-RPC
authentication and response handling, and an authenticated QUIC handshake with
one BAMB frame per stream:

```bash
cargo test --workspace --all-targets --locked
```

These are one-shot examples. Production senders should reuse their HTTP or
QUIC connection and open one QUIC unidirectional stream per bundle.
