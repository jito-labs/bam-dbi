# Jito BAM direct bundle examples

Rust reference clients for submitting ordered bundles directly to Jito BAM by
JSON-RPC or QUIC. Both clients accept one to five fully signed, serialized
Solana transactions in bundle order. The included `send_transfer` examples
build and sign a V0 SOL transfer, so each transport can be tested end to end.

These endpoints are currently for testing. Submitting a signed transaction can
spend real SOL through transaction fees and transfer instructions. Review the
[Jito Terms of Use](https://www.jito.wtf/terms-of-use/) before testing.

## Choose a transport

| Transport | Endpoint | Authentication | Successful client output |
| --- | --- | --- | --- |
| JSON-RPC | `http://<region>.mainnet.bam.jito.wtf:9090/api/v1/bundles` | Jito-provided UUID in `x-jito-auth` | Bundle ID in base64 and hex |
| QUIC | `<region>.mainnet.bam.jito.wtf:11228` | Jito-allowlisted Ed25519 keypair | QUIC stream acknowledgment |

Choose an active region using the
[BAM node explorer](https://explorer.bam.dev/api/v1/validators). The included
environment template uses New York (`ewr`) as an example.

The credentials are separate. A JSON-RPC UUID cannot authenticate a QUIC
connection, and a QUIC keypair cannot authenticate JSON-RPC. New or changed
access can take up to 80 seconds to propagate to every BAM node.

## Prerequisites

- Rust with Cargo. The examples build with stable Rust; repository formatting
  and lint checks use nightly Rust.
- A funded Solana fee-payer keypair for the signed-transfer examples.
- A Jito-provided JSON-RPC UUID, an allowlisted QUIC keypair, or both.
- A Solana RPC URL for fetching a recent blockhash.

Use a dedicated keypair for QUIC authentication. It does not need SOL and
should not be the transaction fee payer or a trading wallet. Share only its
public key with Jito for allowlisting.

## Configure the examples

Copy the environment template and replace the values for the transport you
want to test:

```bash
cp .env.example .env
${EDITOR:-vi} .env
set -a
source .env
set +a
```

The programs read exported environment variables; they do not load `.env`
automatically. The local `.env` file and `*-keypair.json` files are ignored by
Git.

`PAYER_KEYPAIR` must point to a funded Solana JSON keypair. By default, the
signed example sends one lamport back to the payer itself. Set `RECIPIENT` to
send to a different address, and set `LAMPORTS` to change the amount.

## Run the JSON-RPC example

```bash
cargo run --release -p bam-json-rpc --example send_transfer
```

Expected output:

```text
transaction_signature=<base58-transaction-signature>
bundle_id=<base64-bundle-id>
bundle_id_hex=<lowercase-hex-bundle-id>
```

The bundle ID means BAM accepted the request into its ingress queue. It does
not mean the transaction was selected or landed. `bundle_id_hex` is the same
identifier encoded as lowercase hex. Use the printed transaction signature
with a Solana RPC endpoint or explorer to check landing.

## Run the QUIC example

```bash
cargo run --release -p bam-quic --example send_transfer
```

Expected output:

```text
transaction_signature=<base58-transaction-signature>
quic_stream_acknowledged=true
```

The acknowledgment means the authenticated QUIC connection was established,
one BAMB v0 frame was written to a unidirectional stream, and the peer did not
stop that stream with an error. QUIC does not return a bundle ID or landing
result. Use the printed transaction signature to check landing separately.

## Submit an existing signed bundle

Each lower-level client accepts one to five base64-encoded, fully signed
transactions as command-line arguments. Arguments are submitted in bundle
order.

JSON-RPC:

```bash
export SIGNED_TX_1_B64='<base64-signed-transaction>'
cargo run --release -p bam-json-rpc -- "$SIGNED_TX_1_B64"
```

QUIC:

```bash
export SIGNED_TX_1_B64='<base64-signed-transaction>'
cargo run --release -p bam-quic -- "$SIGNED_TX_1_B64"
```

Add up to four more quoted transaction arguments to either command for a
multi-transaction bundle. These lower-level clients do not deserialize the
transactions, so they print the transport result but not transaction
signatures.

## Supported behavior and limits

- Bundles contain one to five ordered transactions and execute atomically.
- JSON-RPC uses HTTP, the `sendBundle` method, base64 transaction encoding, and
  the `/api/v1/bundles` path. HTTPS and base58 encoding are not supported.
- QUIC uses UDP, ALPN `solana-tpu`, and one BAMB v0 frame per unidirectional
  stream.
- The client envelope permits transactions through 4,096 bytes. Keep current
  mainnet transactions within the standard 1,232-byte packet size unless Jito
  explicitly enables a larger transaction format.
- Legacy and versioned transaction support depends on the target cluster. The
  signed-transfer examples use V0.
- Bundle expiry, partial execution, and a bundle-status method are not
  supported. QUIC does not return a bundle ID.
- The initial limit is 25 bundle submissions per second, per credential, per
  BAM node. Start functional testing at one bundle per second.

For production senders, reuse HTTP and QUIC connections rather than creating a
new connection per bundle, and submit to all active BAM regions. The reference
programs are intentionally one-shot examples.

## Test the repository

```bash
cargo +nightly fmt --all -- --check
cargo +nightly clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

The tests cover JSON-RPC authentication and response handling, bundle-ID hex
conversion, BAMB v0 framing, QUIC client authentication, request limits, and
legacy, V0, and V1 serialized transaction fixtures.

## Source layout

- `json-rpc/src/lib.rs`: JSON-RPC `sendBundle` client.
- `json-rpc/examples/send_transfer.rs`: signed V0 transfer over JSON-RPC.
- `quic/src/lib.rs`: authenticated QUIC client and BAMB v0 framing.
- `quic/examples/send_transfer.rs`: signed V0 transfer over QUIC.
