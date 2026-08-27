use std::{
    env,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use quinn::{crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint};
use solana_keypair::{read_keypair_file, Keypair};
use tokio::{net::lookup_host, time::timeout};

const ALPN: &[u8] = b"solana-tpu";
const HEADER_SIZE: usize = 26;
const MAX_TRANSACTIONS: usize = 5;
const MAX_TRANSACTION_SIZE: usize = 4_096;
const TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> Result<()> {
    let frame = encode_bamb_frame(env::args().skip(1).collect())?;
    let address = env::var("BAM_QUIC_ADDR").context("BAM_QUIC_ADDR is required")?;
    let keypair_path = env::var("BAM_QUIC_KEYPAIR").context("BAM_QUIC_KEYPAIR is required")?;
    let keypair = read_keypair_file(&keypair_path)
        .map_err(|error| anyhow!(error.to_string()))
        .with_context(|| format!("read QUIC keypair {keypair_path}"))?;

    let (endpoint, connection) = connect_any(&address, client_config(&keypair)?).await?;
    timeout(TIMEOUT, send_frame(&connection, &frame))
        .await
        .context("timed out sending BAMB frame")??;
    connection.close(0u32.into(), b"bundle sent");
    endpoint.wait_idle().await;

    println!("stream acknowledged; bundle acceptance and landing are unconfirmed");
    Ok(())
}

fn encode_bamb_frame(encoded_transactions: Vec<String>) -> Result<Vec<u8>> {
    ensure!(
        (1..=MAX_TRANSACTIONS).contains(&encoded_transactions.len()),
        "expected 1 to 5 base64-encoded transactions"
    );

    let transactions = encoded_transactions
        .iter()
        .enumerate()
        .map(|(index, encoded)| {
            let transaction = STANDARD
                .decode(encoded)
                .with_context(|| format!("transaction {} is not valid base64", index + 1))?;
            ensure!(
                (1..=MAX_TRANSACTION_SIZE).contains(&transaction.len()),
                "transaction {} must contain 1 to 4096 bytes",
                index + 1
            );
            Ok(transaction)
        })
        .collect::<Result<Vec<_>>>()?;

    let mut frame =
        Vec::with_capacity(HEADER_SIZE + transactions.iter().map(Vec::len).sum::<usize>());
    frame.extend_from_slice(b"BAMB");
    frame.extend_from_slice(&[0, 0, transactions.len() as u8, HEADER_SIZE as u8]);
    frame.extend_from_slice(&0u64.to_le_bytes());
    for transaction in &transactions {
        frame.extend_from_slice(&(transaction.len() as u16).to_le_bytes());
    }
    frame.resize(HEADER_SIZE, 0);
    for transaction in transactions {
        frame.extend_from_slice(&transaction);
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
    use super::*;

    #[test]
    fn encodes_bamb_v0_frame() {
        let frame = encode_bamb_frame(vec!["AQI=".to_owned(), "Aw==".to_owned()]).unwrap();
        assert_eq!(&frame[..4], b"BAMB");
        assert_eq!(&frame[4..8], &[0, 0, 2, 26]);
        assert_eq!(&frame[8..16], &[0; 8]);
        assert_eq!(&frame[16..26], &[2, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&frame[26..], &[1, 2, 3]);

        assert!(encode_bamb_frame(Vec::new()).is_err());
        assert!(encode_bamb_frame(vec!["AQ==".to_owned(); 6]).is_err());
        assert!(encode_bamb_frame(vec!["not-base64".to_owned()]).is_err());
        assert!(encode_bamb_frame(vec![STANDARD.encode(vec![0; 4_097])]).is_err());
    }
}
