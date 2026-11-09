//! `podnet warp register`, `status`, `license`, `rotate` and `delete`: the
//! WARP account and the saved device.

use std::path::PathBuf;

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};

use super::{WarpArgs, WarpCommand};

/// The account subcommands of `podnet warp`.
pub(crate) async fn run(
    args: WarpArgs,
    mut client: podnet::warp::WarpClient,
    path: Option<PathBuf>,
) -> Result<()> {
    let save = |client: &podnet::warp::WarpClient| -> Result<()> {
        if let Some(path) = &path {
            client.save(path)?;
        }
        Ok(())
    };

    match args.command {
        WarpCommand::Register => print_device(&client, path.as_deref()),
        WarpCommand::Status { json } => {
            client.refresh().await?;
            save(&client)?;
            let mut account = client.account().await?;
            // Issue 116: the instance of warp up, with the account.
            let instance = podnet::warp::up::instance_state();
            let tun = podnet::warp::live::instance::instance_state_tun();
            if json {
                let active = podnet::warp::identity::warp_plus_label(&account) == "yes";
                if let Some(doc) = account.as_object_mut() {
                    doc.insert("instance".into(), instance);
                    doc.insert("tun_instance".into(), tun);
                    // Issue 178: warp_plus of the API is not the state of
                    // the account; warp_plus_active is.
                    doc.insert("warp_plus_active".into(), active.into());
                }
                println!("{}", serde_json::to_string_pretty(&account)?);
                return Ok(());
            }
            print_device(&client, path.as_deref());
            println!(
                "warp up     {}",
                podnet::warp::live::instance::instances_line(&instance, &tun)
            );
            let field = |name: &str| account.get(name).map(plain_value).unwrap_or_default();
            println!("type        {}", field("account_type"));
            println!(
                "warp+       {}",
                podnet::warp::identity::warp_plus_label(&account)
            );
            println!("premium     {}", field("premium_data"));
            println!("quota       {}", field("quota"));
        }
        WarpCommand::License { key } => {
            let key = if key == "-" {
                let mut text = String::new();
                std::io::Read::read_to_string(
                    &mut std::io::Read::take(std::io::stdin(), 4096),
                    &mut text,
                )
                .context("reading the license key from stdin")
                .fault(Fault::Usage)?;
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(podnet::exit::fault(
                        Fault::Usage,
                        anyhow::anyhow!("no license key on stdin"),
                    ));
                }
                text
            } else {
                key
            };
            podnet::report::hide(&key);
            let account = client.set_license(&key).await.map_err(|err| {
                podnet::exit::fault(podnet::warp::identity::license_fault(&err), err)
            })?;
            save(&client)?;
            let field = |name: &str| account.get(name).map(plain_value).unwrap_or_default();
            println!("license     set");
            println!("type        {}", field("account_type"));
            println!(
                "warp+       {}",
                podnet::warp::identity::warp_plus_label(&account)
            );
        }
        WarpCommand::Rotate => {
            client.rotate_key().await?;
            save(&client)?;
            println!("key         rotated");
            print_device(&client, path.as_deref());
        }
        WarpCommand::Delete => {
            client.delete().await?;
            if let Some(path) = &path {
                let _ = std::fs::remove_file(path);
            }
            println!("device      {} deleted", client.device().id);
        }
        _ => unreachable!("warp() gives this function its commands only"),
    }
    Ok(())
}

/// A JSON value for a text line: a string with no quotes (issue 178).
pub(crate) fn plain_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "-".into(),
        other => other.to_string(),
    }
}

fn print_device(client: &podnet::warp::WarpClient, path: Option<&std::path::Path>) {
    let device = client.device();
    println!("device      {}", device.id);
    println!("account     {}", device.account.id);
    println!("tunnel      {}", device.tunnel_type);
    println!("key type    {}", device.key_type);
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    println!(
        "ipv4        {}",
        or_dash(device.ipv4().map(|a| a.to_string()))
    );
    println!(
        "ipv6        {}",
        or_dash(device.ipv6().map(|a| a.to_string()))
    );
    println!(
        "masque      {}",
        or_dash(device.masque_endpoint().map(|a| a.to_string()))
    );
    println!("protocol    {}", device.policy.tunnel_protocol);
    match path {
        Some(path) => println!("saved       {}", path.display()),
        None => println!("saved       no (--ephemeral or PODNET_STATE_DIR=none)"),
    }
}
