use crate::{asset::Asset, cli::UploadArgs, config::Creator, web_api::WebApiClient};
use anyhow::Context;
use fs_err::tokio as fs;
use log::info;
use relative_path::PathExt;
use resvg::usvg::fontdb::Database;
use std::{path::PathBuf, sync::Arc};

pub async fn upload(args: UploadArgs) -> anyhow::Result<()> {
    let path = PathBuf::from(&args.path);
    let data = fs::read(&path).await?;

    let mut font_db = Database::new();
    font_db.load_system_fonts();

    let mut asset =
        Asset::new(path.relative_to(".")?, data.into()).context("Failed to create asset")?;

    asset
        .process(Arc::new(font_db), args.bleed)
        .context("Failed to process asset")?;

    // Both or neither: the arguments require each other.
    let creator = match (args.creator_type, args.creator_id) {
        (Some(ty), Some(id)) => Some(Creator { ty, id }),
        _ => None,
    };

    let client = WebApiClient::new(
        args.api_key
            .context("An API key is required to use the upload command")?,
        creator,
        args.expected_price,
    );

    // An update's caller already has the asset ID; what it is waiting for is the version, so
    // an update prints both unless a link was asked for.
    let print_json = args.json || (args.asset_id.is_some() && !args.link);

    let uploaded = match args.asset_id {
        Some(asset_id) => {
            let uploaded = client.update(asset_id, &asset).await?;
            if !print_json {
                match uploaded.version {
                    Some(version) => info!("Asset {asset_id} is now at version {version}"),
                    None => info!("Asset {asset_id} was updated; Roblox did not report a version"),
                }
            }
            uploaded
        }
        None => client.upload(&asset).await?,
    };

    let asset_id = uploaded.asset_id;

    if print_json {
        println!(
            "{}",
            serde_json::json!({ "asset_id": asset_id, "version": uploaded.version })
        );
    } else if args.link {
        println!("https://create.roblox.com/store/asset/{asset_id}");
    } else {
        println!("{asset_id}");
    }

    Ok(())
}
