use anyhow::{bail, ensure, Context};
use bam_direct_bundle_examples::{decode_transactions, encode_bamb_v0};
use quinn::{
    crypto::rustls::QuicClientConfig, ClientConfig, Connection, Endpoint, IdleTimeout,
    TransportConfig,
};
use solana_keypair::read_keypair_file;
use std::{
    env,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{net::lookup_host, time::timeout};

const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const QUIC_MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(2);
const QUIC_KEEP_ALIVE: Duration = Duration::from_secs(1);

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let address = env::var("BAM_QUIC_ADDR").context("BAM_QUIC_ADDR is required")?;
    let keypair_path = env::var("BAM_QUIC_KEYPAIR").context("BAM_QUIC_KEYPAIR is required")?;
    let encoded_transactions = env::args().skip(1).collect::<Vec<_>>();
    let transactions = decode_transactions(&encoded_transactions)?;
    let frame = encode_bamb_v0(&transactions)?;

    println!("transaction_count={}", transactions.len());
    for (index, transaction) in transactions.iter().enumerate() {
        println!("transaction_{}_bytes={}", index + 1, transaction.len());
    }
    println!("bamb_frame_bytes={}", frame.len());

    let keypair = read_keypair_file(&keypair_path)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .with_context(|| format!("read QUIC keypair {keypair_path}"))?;
    let client_config = quic_client_config(&keypair)?;
    let addresses = lookup_host(&address)
        .await
        .with_context(|| format!("resolve BAM_QUIC_ADDR {address}"))?
        .collect::<Vec<_>>();
    ensure!(
        !addresses.is_empty(),
        "BAM_QUIC_ADDR resolved to no addresses"
    );

    let (endpoint, connection) = connect_any(addresses, client_config).await?;
    timeout(SEND_TIMEOUT, send_frame(&connection, &frame))
        .await
        .context("timed out writing the BAMB frame")??;
    connection.close(0u32.into(), b"bundle sent");
    endpoint.wait_idle().await;

    println!("quic_stream=ack (peer acknowledged the stream bytes)");
    println!("quic_send=ok (delivery and landing are unconfirmed)");
    Ok(())
}

fn quic_client_config(keypair: &solana_keypair::Keypair) -> anyhow::Result<ClientConfig> {
    let (certificate, private_key) = solana_tls_utils::new_dummy_x509_certificate(keypair);
    let mut crypto = solana_tls_utils::tls_client_config_builder()
        .with_client_auth_cert(vec![certificate], private_key)
        .context("configure QUIC client certificate")?;
    crypto.enable_early_data = true;
    crypto.alpn_protocols = vec![ALPN_TPU_PROTOCOL_ID.to_vec()];

    let mut client = ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(crypto).context("configure QUIC TLS")?,
    ));
    let mut transport = TransportConfig::default();
    transport.max_idle_timeout(Some(IdleTimeout::try_from(QUIC_MAX_IDLE_TIMEOUT)?));
    transport.keep_alive_interval(Some(QUIC_KEEP_ALIVE));
    transport.send_fairness(false);
    client.transport_config(Arc::new(transport));
    Ok(client)
}

async fn connect_any(
    addresses: Vec<SocketAddr>,
    client_config: ClientConfig,
) -> anyhow::Result<(Endpoint, Connection)> {
    let mut last_error = None;
    for address in addresses {
        match connect(address, client_config.clone()).await {
            Ok(connected) => return Ok(connected),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.context("could not connect to any BAM_QUIC_ADDR address")?)
}

async fn connect(
    address: SocketAddr,
    client_config: ClientConfig,
) -> anyhow::Result<(Endpoint, Connection)> {
    let bind_address: SocketAddr = if address.is_ipv6() {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    };
    let mut endpoint = Endpoint::client(bind_address).context("bind QUIC client socket")?;
    endpoint.set_default_client_config(client_config);
    let server_name = solana_tls_utils::socket_addr_to_quic_server_name(address);
    let connecting = endpoint
        .connect(address, &server_name)
        .with_context(|| format!("start QUIC connection to {address}"))?;
    let connection = timeout(CONNECT_TIMEOUT, connecting)
        .await
        .with_context(|| format!("timed out connecting to {address}"))?
        .with_context(|| format!("connect to {address}"))?;
    Ok((endpoint, connection))
}

async fn send_frame(connection: &Connection, frame: &[u8]) -> anyhow::Result<()> {
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
