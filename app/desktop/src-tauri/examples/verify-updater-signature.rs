use std::{env, fs, path::PathBuf};

use base64::Engine as _;
use minisign_verify::PublicKey;

fn required_arg(args: &[String], name: &str) -> Result<String, String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| format!("missing required argument {name}"))
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let artifact = PathBuf::from(required_arg(&args, "--artifact")?);
    let signature_path = PathBuf::from(required_arg(&args, "--signature")?);
    let configured_public_key = required_arg(&args, "--public-key")?;

    let artifact_bytes = fs::read(&artifact).map_err(|error| {
        format!(
            "read updater artifact {} failed: {error}",
            artifact.display()
        )
    })?;
    let encoded_signature = fs::read_to_string(&signature_path).map_err(|error| {
        format!(
            "read updater signature {} failed: {error}",
            signature_path.display()
        )
    })?;
    let public_key_text = base64::engine::general_purpose::STANDARD
        .decode(configured_public_key.trim())
        .map_err(|error| format!("decode configured updater key base64 failed: {error}"))?;
    let public_key_text = String::from_utf8(public_key_text)
        .map_err(|error| format!("decode configured updater key UTF-8 failed: {error}"))?;
    let public_key = PublicKey::decode(&public_key_text)
        .map_err(|error| format!("parse configured updater public key failed: {error}"))?;
    let signature = shellx_cut_lib::updater_signature::parse_tauri_updater_signature(
        &encoded_signature,
        "updater signature",
    )?;
    public_key
        .verify(&artifact_bytes, &signature, true)
        .map_err(|error| format!("updater signature verification failed: {error}"))?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
