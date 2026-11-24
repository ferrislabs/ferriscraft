use std::path::{Path, PathBuf};
use std::process::Command;

use ferriscraft_orchestrator::s3::{S3Client, S3Config, is_valid_map_name};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("sync") => sync_map().await,
        Some("list") => list_maps().await,
        Some("upload") => {
            if args.len() != 4 {
                return Err(
                    "usage : ferriscraft-map-sync upload <nom-map> <world.zip> <arena.json>"
                        .into(),
                );
            }
            upload_map(&args[1], Path::new(&args[2]), Path::new(&args[3])).await
        }
        Some(other) => {
            Err(format!("sous-commande inconnue '{other}' (attendu : sync | list | upload)").into())
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

async fn sync_map() -> Result<(), Box<dyn std::error::Error>> {
    let Some(s3_cfg) = S3Config::from_env()? else {
        tracing::info!("S3 non configuré (S3_ENDPOINT/S3_BUCKET absents), utilisation du template local");
        return Ok(());
    };
    let client = S3Client::new(s3_cfg)?;

    let requested = ["SHEEPWARS_MAP_NAME", "SHEEPWARS_MAP", "TOWERWARS_MAP_NAME"]
        .into_iter()
        .find_map(non_empty_env);
    let map_name = match requested {
        Some(name) => name,
        None => {
            let maps = client.list_maps().await?;
            maps.into_iter().next().ok_or_else(|| {
                format!(
                    "aucune map complète trouvée dans s3://{}/{}",
                    client.config().bucket,
                    client.config().prefix
                )
            })?
        }
    };

    if !is_valid_map_name(&map_name) {
        return Err(format!("nom de map invalide : {map_name}").into());
    }

    let dest_dir = PathBuf::from(
        non_empty_env("MAP_DEST_DIR").unwrap_or_else(|| "/data".to_owned()),
    );
    tokio::fs::create_dir_all(&dest_dir).await?;

    let arena_key = client.config().map_key(&map_name, "arena.json");
    let world_key = client.config().map_key(&map_name, "world.zip");

    let arena_tmp = dest_dir.join(".arena.json.tmp");
    let arena_dest = dest_dir.join("arena.json");
    let arena_bytes = client.download_to_file(&arena_key, &arena_tmp).await?;
    tokio::fs::rename(&arena_tmp, &arena_dest).await?;

    let zip_tmp = dest_dir.join(".world.zip.tmp");
    let zip_bytes = client.download_to_file(&world_key, &zip_tmp).await?;

    let unpack_dir = dest_dir.join(".map-unpack");
    if unpack_dir.exists() {
        tokio::fs::remove_dir_all(&unpack_dir).await?;
    }
    tokio::fs::create_dir_all(&unpack_dir).await?;

    let status = Command::new("unzip")
        .args(["-q", "-o"])
        .arg(&zip_tmp)
        .arg("-d")
        .arg(&unpack_dir)
        .status()?;
    let _ = tokio::fs::remove_file(&zip_tmp).await;
    if !status.success() {
        return Err(format!("échec extraction unzip (code {:?})", status.code()).into());
    }

    let world_dest = dest_dir.join("world");
    if world_dest.exists() {
        tokio::fs::remove_dir_all(&world_dest).await?;
    }

    let nested_world = unpack_dir.join("world");
    if nested_world.is_dir() {
        tokio::fs::rename(&nested_world, &world_dest).await?;
        let _ = tokio::fs::remove_dir_all(&unpack_dir).await;
    } else if unpack_dir.join("level.dat").is_file() {
        tokio::fs::rename(&unpack_dir, &world_dest).await?;
    } else {
        let _ = tokio::fs::remove_dir_all(&unpack_dir).await;
        return Err("archive world.zip invalide : ni dossier world/ ni level.dat à la racine".into());
    }

    tracing::info!(
        map = %map_name,
        arena_bytes,
        zip_bytes,
        dest = %dest_dir.display(),
        "Map synchronisée depuis S3"
    );
    Ok(())
}

async fn list_maps() -> Result<(), Box<dyn std::error::Error>> {
    let s3_cfg = S3Config::from_env()?.ok_or("S3_ENDPOINT et S3_BUCKET requis")?;
    let client = S3Client::new(s3_cfg)?;
    let maps = client.list_maps().await?;
    for map in maps {
        println!("{map}");
    }
    Ok(())
}

async fn upload_map(
    map_name: &str,
    world_zip: &Path,
    arena_json: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if !is_valid_map_name(map_name) {
        return Err(format!("nom de map invalide : {map_name}").into());
    }
    if !world_zip.is_file() {
        return Err(format!("fichier introuvable : {}", world_zip.display()).into());
    }
    if !arena_json.is_file() {
        return Err(format!("fichier introuvable : {}", arena_json.display()).into());
    }

    let s3_cfg = S3Config::from_env()?.ok_or("S3_ENDPOINT et S3_BUCKET requis")?;
    let client = S3Client::new(s3_cfg)?;
    client.ensure_bucket().await?;

    let arena_key = client.config().map_key(map_name, "arena.json");
    let world_key = client.config().map_key(map_name, "world.zip");

    client
        .put_file(&arena_key, arena_json, "application/json")
        .await?;
    client
        .put_file(&world_key, world_zip, "application/zip")
        .await?;

    tracing::info!(
        map = %map_name,
        bucket = %client.config().bucket,
        world_key = %world_key,
        arena_key = %arena_key,
        "Map téléversée sur S3"
    );
    Ok(())
}
