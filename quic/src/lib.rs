use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use quinn::{crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint};
use solana_keypair::Keypair;
use tokio::{net::lookup_host, time::timeout};

const ALPN: &[u8] = b"solana-tpu";
const HEADER_SIZE: usize = 26;
const MAX_TRANSACTIONS: usize = 5;
const MAX_TRANSACTION_SIZE: usize = 4_096;
const TIMEOUT: Duration = Duration::from_secs(10);

/// Submit one BAMB v0 frame containing signed wire-format transactions.
pub async fn send_bundle(
    address: &str,
    credential: &Keypair,
    transactions: &[Vec<u8>],
) -> Result<()> {
    let frame = encode_bamb_frame(transactions)?;
    let (endpoint, connection) = connect_any(address, client_config(credential)?).await?;
    timeout(TIMEOUT, send_frame(&connection, &frame))
        .await
        .context("timed out sending BAMB frame")??;
    connection.close(0u32.into(), b"bundle sent");
    endpoint.wait_idle().await;
    Ok(())
}

fn encode_bamb_frame(transactions: &[Vec<u8>]) -> Result<Vec<u8>> {
    ensure!(
        (1..=MAX_TRANSACTIONS).contains(&transactions.len()),
        "expected 1 to 5 transactions"
    );
    for (index, transaction) in transactions.iter().enumerate() {
        ensure!(
            (1..=MAX_TRANSACTION_SIZE).contains(&transaction.len()),
            "transaction {} must contain 1 to 4096 bytes",
            index + 1
        );
    }

    let mut frame =
        Vec::with_capacity(HEADER_SIZE + transactions.iter().map(Vec::len).sum::<usize>());
    frame.extend_from_slice(b"BAMB");
    frame.extend_from_slice(&[0, 0, transactions.len() as u8, HEADER_SIZE as u8]);
    frame.extend_from_slice(&0u64.to_le_bytes());
    for transaction in transactions {
        frame.extend_from_slice(&(transaction.len() as u16).to_le_bytes());
    }
    frame.resize(HEADER_SIZE, 0);
    for transaction in transactions {
        frame.extend_from_slice(transaction);
    }
    Ok(frame)
}

fn client_config(keypair: &Keypair) -> Result<ClientConfig> {
    let (certificate, private_key) = solana_tls_utils::new_dummy_x509_certificate(keypair);
    let mut crypto = solana_tls_utils::tls_client_config_builder()
        .with_client_auth_cert(vec![certificate], private_key)
        .context("configure QUIC client certificate")?;
    crypto.alpn_protocols = vec![ALPN.to_vec()];

    Ok(ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(crypto).context("configure QUIC TLS")?,
    )))
}

async fn connect_any(address: &str, config: ClientConfig) -> Result<(Endpoint, Connection)> {
    let addresses = lookup_host(address)
        .await
        .with_context(|| format!("resolve BAM_QUIC_ADDR {address}"))?;
    let mut last_error = None;

    for address in addresses {
        match timeout(TIMEOUT, connect(address, config.clone())).await {
            Ok(Ok(connection)) => return Ok(connection),
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => last_error = Some(anyhow!("timed out connecting to {address}")),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("BAM_QUIC_ADDR resolved to no addresses")))
}

async fn connect(address: SocketAddr, config: ClientConfig) -> Result<(Endpoint, Connection)> {
    let bind_address = if address.is_ipv4() {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
    };
    let mut endpoint = Endpoint::client(bind_address).context("bind QUIC client socket")?;
    endpoint.set_default_client_config(config);
    let server_name = solana_tls_utils::socket_addr_to_quic_server_name(address);
    let connection = endpoint
        .connect(address, &server_name)
        .with_context(|| format!("start QUIC connection to {address}"))?
        .await
        .with_context(|| format!("connect to {address}"))?;
    Ok((endpoint, connection))
}

async fn send_frame(connection: &Connection, frame: &[u8]) -> Result<()> {
    let mut stream = connection
        .open_uni()
        .await
        .context("open unidirectional QUIC stream")?;
    stream.write_all(frame).await.context("write BAMB frame")?;
    stream.finish().context("finish QUIC stream")?;
    if let Some(code) = stream.stopped().await.context("wait for stream status")? {
        bail!("peer stopped QUIC stream with code {code}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use quinn::{crypto::rustls::QuicServerConfig, ServerConfig};
    use solana_hash::Hash;
    use solana_keypair::Signer;
    use solana_message::{v0, v1, VersionedMessage};
    use solana_system_interface::instruction::transfer;
    use solana_transaction::{versioned::VersionedTransaction, Transaction};

    use super::*;

    fn transactions() -> Vec<Vec<u8>> {
        let payer = Keypair::new();
        let blockhash = Hash::new_from_array([7; 32]);
        let v0_message = v0::Message::try_compile(
            &payer.pubkey(),
            &[transfer(&payer.pubkey(), &payer.pubkey(), 1)],
            &[],
            blockhash.clone(),
        )
        .unwrap();
        let v1_message = v1::Message::try_compile(
            &payer.pubkey(),
            &[transfer(&payer.pubkey(), &payer.pubkey(), 2)],
            blockhash.clone(),
        )
        .unwrap();
        let legacy_transaction: VersionedTransaction = Transaction::new_signed_with_payer(
            &[transfer(&payer.pubkey(), &payer.pubkey(), 3)],
            Some(&payer.pubkey()),
            &[&payer],
            blockhash,
        )
        .into();
        [
            VersionedTransaction::try_new(VersionedMessage::V0(v0_message), &[&payer]).unwrap(),
            VersionedTransaction::try_new(VersionedMessage::V1(v1_message), &[&payer]).unwrap(),
            legacy_transaction,
        ]
        .iter()
        .map(|transaction| wincode::serialize(transaction).unwrap())
        .collect()
    }

    #[test]
    fn encodes_v0_v1_and_legacy_transactions() {
        let transactions = transactions();
        let frame = encode_bamb_frame(&transactions).unwrap();

        assert_eq!(&frame[..4], b"BAMB");
        assert_eq!(&frame[4..8], &[0, 0, 3, 26]);
        assert_eq!(&frame[8..16], &[0; 8]);
        for (index, transaction) in transactions.iter().enumerate() {
            let offset = 16 + index * 2;
            assert_eq!(
                &frame[offset..offset + 2],
                &(transaction.len() as u16).to_le_bytes()
            );
        }
        assert_eq!(&frame[22..HEADER_SIZE], &[0; 4]);
        assert_eq!(&frame[HEADER_SIZE..], transactions.concat());

        assert!(encode_bamb_frame(&[]).is_err());
        assert!(encode_bamb_frame(&vec![vec![1]; 6]).is_err());
        assert!(encode_bamb_frame(&[vec![]]).is_err());
        assert!(encode_bamb_frame(&[vec![0; 4_097]]).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sends_authenticated_bundle_on_one_stream() {
        let credential = Keypair::new();
        let transactions = transactions();
        let expected_frame = encode_bamb_frame(&transactions).unwrap();
        let (address, received) = accept_frame().await;

        timeout(
            Duration::from_secs(5),
            send_bundle(&address.to_string(), &credential, &transactions),
        )
        .await
        .expect("QUIC send timed out")
        .unwrap();
        let (pubkey, frame) = timeout(Duration::from_secs(5), received)
            .await
            .expect("server did not receive a frame")
            .unwrap();

        assert_eq!(pubkey, credential.pubkey().to_bytes());
        assert_eq!(frame, expected_frame);
    }

    async fn accept_frame() -> (
        SocketAddr,
        tokio::sync::oneshot::Receiver<([u8; 32], Vec<u8>)>,
    ) {
        let (certificate, private_key) =
            solana_tls_utils::new_dummy_x509_certificate(&Keypair::new());
        let mut crypto = solana_tls_utils::tls_server_config_builder()
            .with_single_cert(vec![certificate], private_key)
            .unwrap();
        crypto.alpn_protocols = vec![b"solana-tpu".to_vec()];
        let config =
            ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(crypto).unwrap()));
        let endpoint = Endpoint::server(config, "127.0.0.1:0".parse().unwrap()).unwrap();
        let address = endpoint.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let connection = endpoint.accept().await.unwrap().await.unwrap();
            let certificates = connection
                .peer_identity()
                .unwrap()
                .downcast::<Vec<quinn::rustls::pki_types::CertificateDer<'static>>>()
                .unwrap();
            let pubkey = solana_tls_utils::get_pubkey_from_tls_certificate(&certificates[0])
                .unwrap()
                .to_bytes();
            let mut stream = connection.accept_uni().await.unwrap();
            let frame = stream
                .read_to_end(HEADER_SIZE + MAX_TRANSACTIONS * MAX_TRANSACTION_SIZE)
                .await
                .unwrap();
            let _ = sender.send((pubkey, frame));
            drop(endpoint);
        });

        (address, receiver)
    }
}
