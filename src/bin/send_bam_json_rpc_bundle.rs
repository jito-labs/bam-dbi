use anyhow::{bail, ensure, Context};
use bam_direct_bundle_examples::send_bundle_request;
use reqwest::{redirect::Policy, Url};
use serde_json::Value;
use std::{env, time::Duration};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let url = env::var("BAM_BUNDLE_URL").context("BAM_BUNDLE_URL is required")?;
    let auth_uuid = env::var("JITO_AUTH_UUID").context("JITO_AUTH_UUID is required")?;
    ensure!(!auth_uuid.is_empty(), "JITO_AUTH_UUID must not be empty");

    let parsed_url = Url::parse(&url).context("BAM_BUNDLE_URL must be a valid URL")?;
    ensure!(
        parsed_url.scheme() == "http" && parsed_url.host().is_some(),
        "BAM_BUNDLE_URL must be an http:// URL"
    );

    let transactions = env::args().skip(1).collect::<Vec<_>>();
    let request = send_bundle_request(&transactions)?;
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("build HTTP client")?;
    let response = client
        .post(parsed_url)
        .header("x-jito-auth", auth_uuid)
        .json(&request)
        .send()
        .await
        .context("could not reach BAM")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("could not read BAM response")?;
    if !status.is_success() {
        bail!("BAM returned HTTP {}: {}", status.as_u16(), body.trim());
    }

    let payload: Value = serde_json::from_str(&body).context("BAM returned a non-JSON response")?;
    ensure!(
        payload.is_object(),
        "BAM returned an invalid JSON-RPC response"
    );
    if let Some(error) = payload.get("error").filter(|error| !error.is_null()) {
        bail!("BAM JSON-RPC error: {error}");
    }
    let bundle_id = payload
        .get("result")
        .and_then(Value::as_str)
        .filter(|bundle_id| !bundle_id.is_empty())
        .context("BAM response did not contain a bundle ID")?;

    println!("bundle_id={bundle_id}");
    println!("accepted_by_ingress=true (this does not guarantee the bundle will land)");
    Ok(())
}
