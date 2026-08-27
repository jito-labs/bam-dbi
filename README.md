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

Add another transport, such as QUIC, as a sibling workspace crate. Share code
only when two transports require the same behavior.
