use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, HOST, HeaderMap, HeaderValue};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Debug, Error)]
pub enum S3Error {
    #[error("configuration S3 incomplète : {0}")]
    IncompleteConfig(String),
    #[error("URL endpoint S3 invalide ({0}) : {1}")]
    InvalidEndpoint(String, #[source] url::ParseError),
    #[error("requête S3 échouée : {0}")]
    Http(#[from] reqwest::Error),
    #[error("erreur S3 ({status}) sur {action} : {body}")]
    Remote {
        action: String,
        status: u16,
        body: String,
    },
    #[error("erreur E/S S3 : {0}")]
    Io(#[from] std::io::Error),
    #[error("valeur d'en-tête HTTP invalide : {0}")]
    InvalidHeader(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub path_style: bool,
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub fn normalize_prefix(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

pub fn is_valid_map_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl S3Config {
    pub fn from_env() -> Result<Option<Self>, S3Error> {
        let endpoint = non_empty_env("S3_ENDPOINT");
        let bucket = non_empty_env("S3_BUCKET");

        match (endpoint, bucket) {
            (None, None) => Ok(None),
            (Some(_), None) => Err(S3Error::IncompleteConfig(
                "S3_ENDPOINT défini sans S3_BUCKET".to_owned(),
            )),
            (None, Some(_)) => Err(S3Error::IncompleteConfig(
                "S3_BUCKET défini sans S3_ENDPOINT".to_owned(),
            )),
            (Some(endpoint), Some(bucket)) => {
                let access_key_id = non_empty_env("S3_ACCESS_KEY_ID").ok_or_else(|| {
                    S3Error::IncompleteConfig("S3_ACCESS_KEY_ID manquant".to_owned())
                })?;
                let secret_access_key = non_empty_env("S3_SECRET_ACCESS_KEY").ok_or_else(|| {
                    S3Error::IncompleteConfig("S3_SECRET_ACCESS_KEY manquant".to_owned())
                })?;
                let prefix = normalize_prefix(
                    non_empty_env("S3_PREFIX")
                        .as_deref()
                        .unwrap_or("maps/"),
                );
                let region =
                    non_empty_env("S3_REGION").unwrap_or_else(|| "us-east-1".to_owned());
                let path_style = !matches!(
                    non_empty_env("S3_PATH_STYLE")
                        .as_deref()
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("false" | "0" | "no")
                );
                Ok(Some(Self {
                    endpoint: endpoint.trim_end_matches('/').to_owned(),
                    bucket,
                    prefix,
                    region,
                    access_key_id,
                    secret_access_key,
                    path_style,
                }))
            }
        }
    }

    pub fn map_key(&self, map_name: &str, file_name: &str) -> String {
        format!("{}{map_name}/{file_name}", self.prefix)
    }
}

#[derive(Debug, Default, Clone)]
pub struct MapCatalog {
    maps: Vec<String>,
    next_index: usize,
}

impl MapCatalog {
    pub fn maps(&self) -> &[String] {
        &self.maps
    }

    pub fn update(&mut self, mut new_maps: Vec<String>) -> bool {
        new_maps.sort();
        new_maps.dedup();
        if new_maps == self.maps {
            return false;
        }
        self.maps = new_maps;
        if !self.maps.is_empty() {
            self.next_index %= self.maps.len();
        } else {
            self.next_index = 0;
        }
        true
    }

    pub fn next_map(&mut self) -> Option<&str> {
        if self.maps.is_empty() {
            return None;
        }
        let idx = self.next_index % self.maps.len();
        self.next_index = (idx + 1) % self.maps.len();
        self.maps.get(idx).map(String::as_str)
    }
}

#[derive(Clone)]
pub struct S3Client {
    config: S3Config,
    base_url: Url,
    http: Client,
}

impl S3Client {
    pub fn new(config: S3Config) -> Result<Self, S3Error> {
        let base_url = Url::parse(&config.endpoint)
            .map_err(|err| S3Error::InvalidEndpoint(config.endpoint.clone(), err))?;
        let http = Client::builder().build()?;
        Ok(Self {
            config,
            base_url,
            http,
        })
    }

    pub fn config(&self) -> &S3Config {
        &self.config
    }

    pub async fn list_maps(&self) -> Result<Vec<String>, S3Error> {
        let mut keys = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut query = vec![("list-type".to_owned(), "2".to_owned())];
            if !self.config.prefix.is_empty() {
                query.push(("prefix".to_owned(), self.config.prefix.clone()));
            }
            if let Some(token) = &continuation_token {
                query.push(("continuation-token".to_owned(), token.clone()));
            }

            let resp = self
                .send_signed("GET", "", &query, Vec::new(), None, Utc::now())
                .await?;
            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                return Err(S3Error::Remote {
                    action: "ListObjectsV2".to_owned(),
                    status: status.as_u16(),
                    body,
                });
            }

            let page = parse_list_objects_v2(&body);
            keys.extend(page.keys);
            if page.is_truncated {
                continuation_token = page.next_continuation_token;
                if continuation_token.is_none() {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(extract_complete_maps(&keys, &self.config.prefix))
    }

    pub async fn download_to_file(&self, key: &str, dest: &Path) -> Result<u64, S3Error> {
        let resp = self
            .send_signed("GET", key, &[], Vec::new(), None, Utc::now())
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(S3Error::Remote {
                action: format!("GetObject({key})"),
                status: status.as_u16(),
                body,
            });
        }

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = resp.bytes_stream();
        let mut total = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            total += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(total)
    }

    pub async fn ensure_bucket(&self) -> Result<(), S3Error> {
        let resp = self
            .send_signed("PUT", "", &[], Vec::new(), None, Utc::now())
            .await?;
        let status = resp.status();
        if status.is_success() || status.as_u16() == 409 {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        if status.as_u16() == 409
            || body.contains("BucketAlreadyOwnedByYou")
            || body.contains("BucketAlreadyExists")
        {
            return Ok(());
        }
        Err(S3Error::Remote {
            action: format!("CreateBucket({})", self.config.bucket),
            status: status.as_u16(),
            body,
        })
    }

    pub async fn put_file(
        &self,
        key: &str,
        src: &Path,
        content_type: &str,
    ) -> Result<(), S3Error> {
        let bytes = tokio::fs::read(src).await?;
        let resp = self
            .send_signed("PUT", key, &[], bytes, Some(content_type), Utc::now())
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(S3Error::Remote {
                action: format!("PutObject({key})"),
                status: status.as_u16(),
                body,
            });
        }
        Ok(())
    }

    async fn send_signed(
        &self,
        method: &str,
        key: &str,
        query: &[(String, String)],
        body: Vec<u8>,
        content_type: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<reqwest::Response, S3Error> {
        let signed = build_signed_request(
            &self.config,
            &self.base_url,
            method,
            key,
            query,
            &body,
            now,
        )?;
        let mut headers = HeaderMap::new();
        for (name, value) in &signed.headers {
            let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| S3Error::InvalidHeader(err.to_string()))?;
            let header_val = HeaderValue::from_str(value)
                .map_err(|err| S3Error::InvalidHeader(err.to_string()))?;
            headers.insert(header_name, header_val);
        }
        if let Some(ct) = content_type {
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_str(ct).map_err(|err| S3Error::InvalidHeader(err.to_string()))?,
            );
        }
        let req_method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|err| S3Error::InvalidHeader(err.to_string()))?;
        let mut req = self.http.request(req_method, signed.url).headers(headers);
        if !body.is_empty() {
            req = req.body(body);
        }
        Ok(req.send().await?)
    }
}

#[derive(Debug)]
struct SignedRequest {
    url: Url,
    headers: Vec<(String, String)>,
}

fn build_signed_request(
    config: &S3Config,
    base_url: &Url,
    method: &str,
    key: &str,
    query: &[(String, String)],
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<SignedRequest, S3Error> {
    let mut url = base_url.clone();
    let base_host = url
        .host_str()
        .ok_or_else(|| S3Error::InvalidHeader("endpoint sans hôte".to_owned()))?
        .to_owned();

    let clean_key = key.trim_start_matches('/');
    let base_path = url.path().trim_end_matches('/').to_owned();

    let (host_header, canonical_uri) = if config.path_style {
        let host = match url.port() {
            Some(port) => format!("{base_host}:{port}"),
            None => base_host,
        };
        let raw_path = if clean_key.is_empty() {
            format!("{base_path}/{}", config.bucket)
        } else {
            format!("{base_path}/{}/{}", config.bucket, clean_key)
        };
        (host, aws_uri_encode(&raw_path, false))
    } else {
        let virtual_host = format!("{}.{}", config.bucket, base_host);
        url.set_host(Some(&virtual_host))
            .map_err(|err| S3Error::InvalidHeader(err.to_string()))?;
        let host = match url.port() {
            Some(port) => format!("{virtual_host}:{port}"),
            None => virtual_host,
        };
        let raw_path = if clean_key.is_empty() {
            if base_path.is_empty() {
                "/".to_owned()
            } else {
                format!("{base_path}/")
            }
        } else {
            format!("{base_path}/{clean_key}")
        };
        (host, aws_uri_encode(&raw_path, false))
    };

    url.set_path(&canonical_uri);

    let mut encoded_query: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (aws_uri_encode(k, true), aws_uri_encode(v, true)))
        .collect();
    encoded_query.sort();
    let canonical_query = encoded_query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    if canonical_query.is_empty() {
        url.set_query(None);
    } else {
        url.set_query(Some(&canonical_query));
    }

    let payload_hash = if body.is_empty() {
        EMPTY_SHA256.to_owned()
    } else {
        sha256_hex(body)
    };
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let canonical_headers = format!(
        "host:{host_header}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"
    );
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );

    let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", config.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac_sha256(
        format!("AWS4{}", config.secret_access_key).as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, config.region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex_lower(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        config.access_key_id
    );

    Ok(SignedRequest {
        url,
        headers: vec![
            (HOST.as_str().to_owned(), host_header),
            ("x-amz-content-sha256".to_owned(), payload_hash),
            ("x-amz-date".to_owned(), amz_date),
            ("authorization".to_owned(), authorization),
        ],
    })
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex_lower(&digest)
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_LEN: usize = 64;
    let mut k_pad = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        let hashed = Sha256::digest(key);
        k_pad[..32].copy_from_slice(&hashed);
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_LEN];
    let mut opad = [0x5cu8; BLOCK_LEN];
    for i in 0..BLOCK_LEN {
        ipad[i] ^= k_pad[i];
        opad[i] ^= k_pad[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn aws_uri_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let is_unreserved = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (!encode_slash && byte == b'/');
        if is_unreserved {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
struct ListObjectsPage {
    keys: Vec<String>,
    is_truncated: bool,
    next_continuation_token: Option<String>,
}

fn parse_list_objects_v2(xml: &str) -> ListObjectsPage {
    let mut keys = Vec::new();
    let mut cursor = xml;
    while let Some(start) = cursor.find("<Contents>") {
        let after_start = &cursor[start + "<Contents>".len()..];
        let Some(end) = after_start.find("</Contents>") else {
            break;
        };
        let block = &after_start[..end];
        if let Some(key) = extract_xml_tag(block, "Key") {
            keys.push(unescape_xml(&key));
        }
        cursor = &after_start[end + "</Contents>".len()..];
    }

    let is_truncated = extract_xml_tag(xml, "IsTruncated")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
    let next_continuation_token =
        extract_xml_tag(xml, "NextContinuationToken").map(|v| unescape_xml(&v));

    ListObjectsPage {
        keys,
        is_truncated,
        next_continuation_token,
    }
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].to_owned())
}

fn unescape_xml(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

pub fn extract_complete_maps(keys: &[String], prefix: &str) -> Vec<String> {
    let mut seen: HashMap<&str, (bool, bool)> = HashMap::new();
    for key in keys {
        let Some(rest) = key.strip_prefix(prefix) else {
            continue;
        };
        let Some((map_name, file_name)) = rest.split_once('/') else {
            continue;
        };
        if !is_valid_map_name(map_name) || file_name.contains('/') {
            continue;
        }
        let entry = seen.entry(map_name).or_insert((false, false));
        match file_name {
            "world.zip" => entry.0 = true,
            "arena.json" => entry.1 = true,
            _ => {}
        }
    }

    let mut maps: Vec<String> = seen
        .into_iter()
        .filter_map(|(name, (has_world, has_arena))| {
            if has_world && has_arena {
                Some(name.to_owned())
            } else {
                None
            }
        })
        .collect();
    maps.sort();
    maps
}

pub fn sanitize_k8s_label(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(63)
        .collect();
    sanitized
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_owned()
}

pub fn build_url_list_summary(maps: &[String]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for m in maps {
        *counts.entry(m.as_str()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::Query;
    use axum::routing::get;
    use chrono::TimeZone;

    #[test]
    fn extracts_only_complete_maps_sorted() {
        let keys = vec![
            "maps/steampunk/world.zip".to_owned(),
            "maps/steampunk/arena.json".to_owned(),
            "maps/pirate/world.zip".to_owned(),
            "maps/sky_island/arena.json".to_owned(),
            "maps/sky_island/world.zip".to_owned(),
            "maps/nested/sub/world.zip".to_owned(),
            "other/foo/world.zip".to_owned(),
        ];
        assert_eq!(
            extract_complete_maps(&keys, "maps/"),
            vec!["sky_island".to_owned(), "steampunk".to_owned()]
        );
    }

    #[test]
    fn parses_list_objects_v2_xml_with_pagination_and_entities() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>ferriscraft-maps</Name>
  <Prefix>maps/</Prefix>
  <IsTruncated>true</IsTruncated>
  <NextContinuationToken>tok&amp;1</NextContinuationToken>
  <Contents><Key>maps/a&amp;b/world.zip</Key></Contents>
  <Contents><Key>maps/steampunk/arena.json</Key></Contents>
</ListBucketResult>"#;
        let page = parse_list_objects_v2(xml);
        assert!(page.is_truncated);
        assert_eq!(page.next_continuation_token.as_deref(), Some("tok&1"));
        assert_eq!(
            page.keys,
            vec![
                "maps/a&b/world.zip".to_owned(),
                "maps/steampunk/arena.json".to_owned()
            ]
        );
    }

    #[test]
    fn map_catalog_rotates_round_robin_and_preserves_bounds_on_update() {
        let mut catalog = MapCatalog::default();
        assert_eq!(catalog.next_map(), None);

        assert!(catalog.update(vec!["steampunk".to_owned(), "aztec".to_owned()]));
        assert_eq!(catalog.maps(), &["aztec", "steampunk"]);
        assert_eq!(catalog.next_map(), Some("aztec"));
        assert_eq!(catalog.next_map(), Some("steampunk"));
        assert_eq!(catalog.next_map(), Some("aztec"));

        assert!(!catalog.update(vec!["aztec".to_owned(), "steampunk".to_owned()]));
        assert!(catalog.update(vec!["pirate".to_owned()]));
        assert_eq!(catalog.next_map(), Some("pirate"));
    }

    #[test]
    fn sigv4_produces_deterministic_path_style_signature() {
        let cfg = S3Config {
            endpoint: "http://ferriscraft-minio:9000".to_owned(),
            bucket: "ferriscraft-maps".to_owned(),
            prefix: "maps/".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: "AKIAEXAMPLE".to_owned(),
            secret_access_key: "secret-key".to_owned(),
            path_style: true,
        };
        let base = Url::parse(&cfg.endpoint).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 20, 12, 0, 0).unwrap();
        let signed = build_signed_request(
            &cfg,
            &base,
            "GET",
            "maps/steampunk/arena.json",
            &[],
            &[],
            now,
        )
        .unwrap();

        assert_eq!(
            signed.url.as_str(),
            "http://ferriscraft-minio:9000/ferriscraft-maps/maps/steampunk/arena.json"
        );
        let auth = signed
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(auth.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20261120/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="
        ));
    }

    #[tokio::test]
    async fn client_lists_downloads_and_uploads_against_mock_s3() {
        let app = Router::new()
            .route(
                "/ferriscraft-maps",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    if q.contains_key("continuation-token") {
                        r#"<ListBucketResult><IsTruncated>false</IsTruncated>
                        <Contents><Key>maps/steampunk/arena.json</Key></Contents>
                        <Contents><Key>maps/pirate/world.zip</Key></Contents>
                        <Contents><Key>maps/pirate/arena.json</Key></Contents>
                        </ListBucketResult>"#
                    } else {
                        r#"<ListBucketResult><IsTruncated>true</IsTruncated>
                        <NextContinuationToken>page2</NextContinuationToken>
                        <Contents><Key>maps/steampunk/world.zip</Key></Contents>
                        <Contents><Key>maps/broken/world.zip</Key></Contents>
                        </ListBucketResult>"#
                    }
                })
                .put(|| async { axum::http::StatusCode::OK }),
            )
            .route(
                "/ferriscraft-maps/{*key}",
                get(|| async { b"{\"prebuilt\":true}".to_vec() })
                    .put(|| async { axum::http::StatusCode::OK }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = S3Client::new(S3Config {
            endpoint: format!("http://{addr}"),
            bucket: "ferriscraft-maps".to_owned(),
            prefix: "maps/".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: "test".to_owned(),
            secret_access_key: "secret".to_owned(),
            path_style: true,
        })
        .unwrap();

        client.ensure_bucket().await.unwrap();
        let maps = client.list_maps().await.unwrap();
        assert_eq!(maps, vec!["pirate".to_owned(), "steampunk".to_owned()]);

        let tmp = std::env::temp_dir().join(format!("s3_test_{}.json", uuid::Uuid::new_v4()));
        let bytes = client
            .download_to_file("maps/steampunk/arena.json", &tmp)
            .await
            .unwrap();
        assert!(bytes > 0);
        assert_eq!(std::fs::read(&tmp).unwrap(), b"{\"prebuilt\":true}");
        client
            .put_file("maps/steampunk/arena.json", &tmp, "application/json")
            .await
            .unwrap();
        let _ = std::fs::remove_file(&tmp);
    }
}
