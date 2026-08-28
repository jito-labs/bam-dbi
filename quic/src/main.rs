use std::env;

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use solana_keypair::read_keypair_file;

#[tokio::main]
async fn main() -> Result<()> {
    let transactions = env::args()
        .skip(1)
        .enumerate()
        .map(|(index, transaction)| {
            STANDARD
                .decode(transaction)
                .with_context(|| format!("transaction {} is not valid base64", index + 1))
        })
        .collect::<Result<Vec<_>>>()?;
    let address = env::var("BAM_QUIC_ADDR").context("BAM_QUIC_ADDR is required")?;
    let keypair_path = env::var("BAM_QUIC_KEYPAIR").context("BAM_QUIC_KEYPAIR is required")?;
    let keypair = read_keypair_file(&keypair_path)
        .map_err(|error| anyhow!(error.to_string()))
        .with_context(|| format!("read QUIC keypair {keypair_path}"))?;

    bam_quic::send_bundle(&address, &keypair, &transactions).await?;

    println!("quic_stream_acknowledged=true");
    Ok(())
}
