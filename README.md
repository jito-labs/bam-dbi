# BAM direct bundle examples

Small clients for submitting bundles directly to BAM. Each transport is an
independent Cargo workspace crate so its dependencies stay isolated.

## JSON-RPC

The `json-rpc` crate sends one to five already-signed, base64-encoded
transactions in bundle order:

```bash
export BAM_BUNDLE_URL='<provided-bam-url>/api/v1/bundles'
export JITO_AUTH_UUID='<provided-uuid>'
export SIGNED_TX_B64='<base64-signed-transaction>'

cargo run --release -p bam-json-rpc -- "$SIGNED_TX_B64"
```

A returned bundle ID means BAM accepted the bundle into its ingress queue. It
does not guarantee the bundle will land.

## QUIC

The `quic` crate sends the same input as one BAMB v0 frame. Its keypair is the
allowlisted QUIC credential, not a transaction signer:

```bash
export BAM_QUIC_ADDR='<provided-host>:<port>'
export BAM_QUIC_KEYPAIR='/path/to/allowlisted-keypair.json'

cargo run --release -p bam-quic -- "$SIGNED_TX_B64"
```

A successful send means the peer acknowledged the stream bytes. QUIC returns
no bundle ID or application response, so acceptance and landing are
unconfirmed. Production senders should reuse the connection and open one
unidirectional stream per bundle.
