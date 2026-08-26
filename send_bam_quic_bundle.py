#!/usr/bin/env python3
"""Submit one BAMB v0 bundle over authenticated QUIC to Jito BAM."""

import argparse
import asyncio
import base64
import binascii
import json
import os
import ssl
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from urllib.parse import urlsplit

from aioquic.asyncio import connect
from aioquic.quic.configuration import QuicConfiguration
from cryptography import x509
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ed25519
from cryptography.x509.oid import NameOID

MAX_TXS_PER_BUNDLE = 5
STANDARD_PACKET_SIZE = 1232


def decode_transactions(transactions: list[str]) -> list[bytes]:
    if not 1 <= len(transactions) <= MAX_TXS_PER_BUNDLE:
        raise ValueError("a bundle must contain 1 to 5 transactions")

    decoded_transactions = []
    for index, transaction in enumerate(transactions, start=1):
        try:
            decoded = base64.b64decode(transaction, validate=True)
        except (binascii.Error, ValueError) as error:
            raise ValueError(f"transaction {index} must be valid base64") from error
        if not decoded:
            raise ValueError(f"transaction {index} must be non-empty")
        if len(decoded) > STANDARD_PACKET_SIZE:
            raise ValueError(
                f"transaction {index} exceeds the standard 1232-byte packet size"
            )
        decoded_transactions.append(decoded)
    return decoded_transactions


def encode_bamb_v0(signed_transactions: list[bytes]) -> bytes:
    if not 1 <= len(signed_transactions) <= MAX_TXS_PER_BUNDLE:
        raise ValueError("a bundle must contain 1 to 5 transactions")
    if any(not transaction for transaction in signed_transactions):
        raise ValueError("transactions must be non-empty")
    if any(
        len(transaction) > STANDARD_PACKET_SIZE
        for transaction in signed_transactions
    ):
        raise ValueError(
            "serialized transaction exceeds the standard 1232-byte packet size"
        )

    header = bytearray(b"BAMB")
    header += bytes([0, 0, len(signed_transactions), 26])
    header += (0).to_bytes(8, "little")
    for transaction in signed_transactions:
        header += len(transaction).to_bytes(2, "little")
    header += bytes(2 * (MAX_TXS_PER_BUNDLE - len(signed_transactions)))
    return bytes(header) + b"".join(signed_transactions)


def read_quic_private_key(keypair_path: str) -> ed25519.Ed25519PrivateKey:
    try:
        values = json.loads(Path(keypair_path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"could not read QUIC keypair: {error}") from error
    if (
        not isinstance(values, list)
        or len(values) != 64
        or any(type(value) is not int or not 0 <= value <= 255 for value in values)
    ):
        raise ValueError("expected a 64-byte Solana keypair JSON array")

    raw = bytes(values)
    private_key = ed25519.Ed25519PrivateKey.from_private_bytes(raw[:32])
    derived_public_key = private_key.public_key().public_bytes(
        serialization.Encoding.Raw,
        serialization.PublicFormat.Raw,
    )
    if raw[32:] != derived_public_key:
        raise ValueError("QUIC keypair public key does not match its private seed")
    return private_key


def write_cert_chain(
    directory: str,
    private_key: ed25519.Ed25519PrivateKey,
) -> tuple[str, str]:
    common_name = x509.NameAttribute(NameOID.COMMON_NAME, "bam-quic-client")
    subject = issuer = x509.Name([common_name])
    now = datetime.now(timezone.utc)
    certificate = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(private_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - timedelta(minutes=5))
        .not_valid_after(now + timedelta(days=365))
        .sign(private_key, algorithm=None)
    )
    cert_path = Path(directory, "client-cert.pem")
    key_path = Path(directory, "client-key.pem")
    cert_path.write_bytes(certificate.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(
        private_key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
    )
    return str(cert_path), str(key_path)


def split_host_port(address: str) -> tuple[str, int]:
    parsed = urlsplit(f"//{address}")
    try:
        port = parsed.port
    except ValueError as error:
        raise ValueError(f"invalid QUIC address: {error}") from error
    if not parsed.hostname or port is None or parsed.path:
        raise ValueError("QUIC address must be host:port")
    return parsed.hostname, port


async def send_bundle(
    address: str,
    keypair_path: str,
    encoded_transactions: list[str],
) -> None:
    host, port = split_host_port(address)
    transactions = decode_transactions(encoded_transactions)
    frame = encode_bamb_v0(transactions)

    print(f"transaction_count={len(transactions)}")
    for index, transaction in enumerate(transactions, start=1):
        print(f"transaction_{index}_bytes={len(transaction)}")
    print(f"bamb_frame_bytes={len(frame)}")

    private_key = read_quic_private_key(keypair_path)
    with tempfile.TemporaryDirectory(prefix="bam-quic-") as directory:
        cert_path, key_path = write_cert_chain(directory, private_key)
        configuration = QuicConfiguration(
            is_client=True,
            alpn_protocols=["solana-tpu"],
        )
        # The server certificate is ephemeral; the client side is authenticated.
        configuration.verify_mode = ssl.CERT_NONE
        configuration.server_name = host
        configuration.load_cert_chain(cert_path, key_path)

        async with connect(host, port, configuration=configuration) as client:
            _reader, writer = await client.create_stream(is_unidirectional=True)
            writer.write(frame)
            writer.write_eof()
            await writer.drain()
            await asyncio.wait_for(client.ping(), timeout=3.0)
            await asyncio.sleep(0.25)

    print("quic_ping=ack (peer reachable after stream write)")
    print("quic_send=ok (write completed; delivery and landing are unconfirmed)")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Submit 1-5 base64 signed transactions as one BAMB v0 QUIC bundle."
    )
    parser.add_argument(
        "--address",
        default=os.environ.get("BAM_QUIC_ADDR"),
        help="BAM QUIC host:port (or set BAM_QUIC_ADDR)",
    )
    parser.add_argument(
        "--quic-keypair",
        default=os.environ.get("BAM_QUIC_KEYPAIR"),
        help="allowlisted QUIC auth keypair JSON (or set BAM_QUIC_KEYPAIR)",
    )
    parser.add_argument(
        "transaction",
        nargs="+",
        help="base64 signed transaction, 1 to 5 values",
    )
    args = parser.parse_args()

    if not args.address:
        parser.error("--address or BAM_QUIC_ADDR is required")
    if not args.quic_keypair:
        parser.error("--quic-keypair or BAM_QUIC_KEYPAIR is required")
    try:
        asyncio.run(send_bundle(args.address, args.quic_keypair, args.transaction))
    except (asyncio.TimeoutError, OSError, RuntimeError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
