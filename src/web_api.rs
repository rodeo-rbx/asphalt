use crate::{
    asset::{Asset, AssetType},
    config,
};
use anyhow::{Context, bail};
use log::{debug, warn};
use reqwest::{Method, RequestBuilder, Response, StatusCode, multipart};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    env, fmt,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::Mutex;
use tokio::time::Instant;

const RATELIMIT_RESET_HEADER: &str = "x-ratelimit-reset";

const UPLOAD_URL: &str = "https://apis.roblox.com/assets/v1/assets";
const OPERATION_URL: &str = "https://apis.roblox.com/assets/v1/operations";
const ASSET_DESCRIPTION: &str = "Uploaded by Asphalt";
const MAX_DISPLAY_NAME_LENGTH: usize = 50;

/// What Roblox reports once an upload has been processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadedAsset {
    pub asset_id: u64,
    /// The version number of the uploaded content within its asset (1 for a new asset), when
    /// Roblox reports one. This is the number `rbxassetid://<id>&version=<n>` takes, not the
    /// global asset version ID that `InsertService:LoadAssetVersion` takes.
    pub version: Option<u64>,
}

pub struct WebApiClient {
    inner: reqwest::Client,
    api_key: String,
    /// Required to create an asset; optional when updating one.
    creator: Option<config::Creator>,
    expected_price: Option<u32>,
    fatally_failed: AtomicBool,
    /// Shared rate limit state: when we can next make a request
    rate_limit_reset: Mutex<Option<Instant>>,
}

impl WebApiClient {
    pub fn new(
        api_key: String,
        creator: Option<config::Creator>,
        expected_price: Option<u32>,
    ) -> Self {
        WebApiClient {
            inner: reqwest::Client::new(),
            api_key,
            creator,
            expected_price,
            fatally_failed: AtomicBool::new(false),
            rate_limit_reset: Mutex::new(None),
        }
    }

    /// Creates a new asset from the file.
    pub async fn upload(&self, asset: &Asset) -> anyhow::Result<UploadedAsset> {
        if env::var("ASPHALT_TEST").is_ok() {
            return Ok(UploadedAsset {
                asset_id: asset.hash.as_u64(),
                version: Some(1),
            });
        }

        let creator = self
            .creator
            .clone()
            .context("A creator is required to create an asset")?;

        let file_name = asset.path.file_name().unwrap();
        let display_name = trim_display_name(file_name);

        let req = Request {
            display_name,
            asset_type: asset.ty,
            creation_context: CreationContext {
                creator: creator.into(),
                expected_price: self.expected_price,
            },
            description: ASSET_DESCRIPTION,
        };

        let response = self
            .send_asset(
                Method::POST,
                UPLOAD_URL.to_owned(),
                serde_json::to_string(&req)?,
                asset,
            )
            .await?;

        Ok(UploadedAsset {
            asset_id: response
                .asset_id
                .context("Operation completed but returned no asset ID")?,
            version: response.revision_id,
        })
    }

    /// Uploads the file as a new version of an existing asset. The asset keeps its ID, name and
    /// description. Roblox decides which asset types can be updated this way.
    pub async fn update(&self, asset_id: u64, asset: &Asset) -> anyhow::Result<UploadedAsset> {
        if env::var("ASPHALT_TEST").is_ok() {
            return Ok(UploadedAsset {
                asset_id,
                version: Some(2),
            });
        }

        let req = UpdateRequest {
            asset_type: asset.ty,
            asset_id: asset_id.to_string(),
            creation_context: self.creator.clone().map(|creator| CreationContext {
                creator: creator.into(),
                expected_price: self.expected_price,
            }),
        };

        let response = self
            .send_asset(
                Method::PATCH,
                format!("{UPLOAD_URL}/{asset_id}"),
                serde_json::to_string(&req)?,
                asset,
            )
            .await?;

        // By now the new version exists, so a response that names another asset is reported
        // rather than passed on as if it were the asset that was asked for.
        if let Some(reported) = response.asset_id
            && reported != asset_id
        {
            bail!("Roblox reported asset {reported} for an update to asset {asset_id}");
        }

        Ok(UploadedAsset {
            asset_id,
            version: response.revision_id,
        })
    }

    /// Sends the file with its request metadata, then waits for Roblox to finish processing it.
    async fn send_asset(
        &self,
        method: Method,
        url: String,
        req_json: String,
        asset: &Asset,
    ) -> anyhow::Result<OperationResponse> {
        let len = asset.data.len() as u64;
        let mime = asset.ty.file_type().to_owned();
        let name = asset.path.file_name().unwrap().to_owned();

        let res = self
            .send_with_retry(|client| {
                let file_part = multipart::Part::stream_with_length(
                    reqwest::Body::from(asset.data.clone()),
                    len,
                )
                .file_name(name.clone())
                .mime_str(&mime)
                .unwrap();

                let form = multipart::Form::new()
                    .text("request", req_json.clone())
                    .part("fileContent", file_part);

                client
                    .request(method.clone(), &url)
                    .header("x-api-key", &self.api_key)
                    .multipart(form)
            })
            .await?;

        let body = res.text().await?;

        let operation: Operation = serde_json::from_str(&body)?;
        let operation_id = operation
            .id()
            .with_context(|| format!("Upload accepted but no operation ID in response: {body}"))?;

        self.poll_operation(operation_id, &self.api_key)
            .await
            .context("Failed to poll operation")
    }

    async fn poll_operation(&self, id: String, api_key: &str) -> anyhow::Result<OperationResponse> {
        let mut delay = Duration::from_secs(1);
        const MAX_POLLS: u32 = 10;

        for attempt in 0..MAX_POLLS {
            let res = self
                .send_with_retry(|client| {
                    client
                        .get(format!("{OPERATION_URL}/{id}"))
                        .header("x-api-key", api_key)
                })
                .await?;

            let text = res.text().await?;

            let operation: Operation = serde_json::from_str(&text)?;

            if operation.done {
                if let Some(error) = operation.error {
                    bail!("Roblox rejected the upload: {error}");
                }

                return operation
                    .response
                    .context("Operation completed but no response provided");
            }

            debug!("Operation not done yet");

            if attempt < MAX_POLLS - 1 {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }

        bail!("Operation polling exceeded maximum retries")
    }

    async fn send_with_retry<F>(&self, make_req: F) -> anyhow::Result<Response>
    where
        F: Fn(&reqwest::Client) -> RequestBuilder,
    {
        if self.fatally_failed.load(Ordering::SeqCst) {
            bail!("A previous request failed due to a fatal error");
        }

        const MAX: u8 = 5;
        let mut attempt = 0;

        loop {
            {
                let reset = self.rate_limit_reset.lock().await;
                if let Some(reset_at) = *reset {
                    let now = Instant::now();
                    if reset_at > now {
                        let wait = reset_at - now;
                        drop(reset);
                        debug!("Waiting {:.2}ms for rate limit reset", wait.as_secs_f64());
                        tokio::time::sleep(wait).await;
                    }
                }
            }

            let res = make_req(&self.inner).send().await?;
            let status = res.status();

            match status {
                StatusCode::TOO_MANY_REQUESTS if attempt < MAX => {
                    let wait = res
                        .headers()
                        .get(RATELIMIT_RESET_HEADER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| Duration::from_secs(1 << attempt));

                    let reset_at = Instant::now() + wait;
                    {
                        let mut reset = self.rate_limit_reset.lock().await;
                        *reset = Some(reset_at);
                    }

                    warn!(
                        "Rate limited, retrying in {:.2} seconds",
                        wait.as_secs_f64()
                    );

                    tokio::time::sleep(wait).await;
                    attempt += 1;

                    continue;
                }
                _ if status.is_success() => return Ok(res),
                _ => {
                    let body = res.text().await?;
                    self.fatally_failed.store(true, Ordering::SeqCst);
                    bail!("Request failed with status {status}:\n{body}");
                }
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    asset_type: AssetType,
    display_name: String,
    description: &'static str,
    creation_context: CreationContext,
}

/// The request for a new version of an existing asset. Roblox accepts it with or without the
/// creation context.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest {
    asset_type: AssetType,
    asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    creation_context: Option<CreationContext>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreationContext {
    creator: Creator,
    expected_price: Option<u32>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Creator {
    User(UserCreator),
    Group(GroupCreator),
}

impl From<config::Creator> for Creator {
    fn from(value: config::Creator) -> Self {
        match value.ty {
            config::CreatorType::User => Creator::User(UserCreator {
                user_id: value.id.to_string(),
            }),
            config::CreatorType::Group => Creator::Group(GroupCreator {
                group_id: value.id.to_string(),
            }),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserCreator {
    user_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupCreator {
    group_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Operation {
    #[serde(default)]
    done: bool,
    operation_id: Option<String>,
    /// `operations/<id>`
    path: Option<String>,
    response: Option<OperationResponse>,
    error: Option<OperationError>,
}

impl Operation {
    fn id(&self) -> Option<String> {
        self.operation_id.clone().or_else(|| {
            self.path
                .as_deref()
                .and_then(|path| path.strip_prefix("operations/"))
                .map(str::to_owned)
        })
    }
}

#[derive(Deserialize)]
struct OperationError {
    code: Option<serde_json::Value>,
    message: Option<String>,
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.code, &self.message) {
            (Some(code), Some(message)) => write!(f, "{message} (code {code})"),
            (None, Some(message)) => write!(f, "{message}"),
            (Some(code), None) => write!(f, "code {code}"),
            (None, None) => write!(f, "no details given"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationResponse {
    #[serde(default, deserialize_with = "lenient_u64")]
    asset_id: Option<u64>,
    /// The version number of the uploaded content. Roblox documents it as an integer string.
    #[serde(default, deserialize_with = "lenient_u64")]
    revision_id: Option<u64>,
}

/// Reads an integer sent as a JSON string or number. Anything else reads as absent: by the time
/// this response arrives the upload has gone through, so an unexpected shape must not turn it
/// into a failure that invites a second upload.
fn lenient_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            Some(serde_json::Value::String(text)) => text.parse().ok(),
            Some(serde_json::Value::Number(number)) => number.as_u64(),
            _ => None,
        },
    )
}

fn trim_display_name(name: &str) -> String {
    let full_path = name.to_string();
    if full_path.len() > MAX_DISPLAY_NAME_LENGTH {
        let start_index = full_path.len().saturating_sub(MAX_DISPLAY_NAME_LENGTH);
        full_path[start_index..].to_string()
    } else {
        full_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::ModelType;
    use serde_json::json;

    fn group(id: u64) -> config::Creator {
        config::Creator {
            ty: config::CreatorType::Group,
            id,
        }
    }

    #[test]
    fn update_request_names_the_asset_and_nothing_else() {
        let req = UpdateRequest {
            asset_type: AssetType::Model(ModelType::Roblox),
            asset_id: 139145374182770u64.to_string(),
            creation_context: None,
        };

        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({ "assetType": "Model", "assetId": "139145374182770" })
        );
    }

    #[test]
    fn update_request_carries_the_creator_when_given() {
        let req = UpdateRequest {
            asset_type: AssetType::Model(ModelType::Roblox),
            asset_id: "1".to_string(),
            creation_context: Some(CreationContext {
                creator: group(5499489).into(),
                expected_price: None,
            }),
        };

        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({
                "assetType": "Model",
                "assetId": "1",
                "creationContext": {
                    "creator": { "groupId": "5499489" },
                    "expectedPrice": null,
                },
            })
        );
    }

    #[test]
    fn operation_id_comes_from_either_field() {
        let by_id: Operation =
            serde_json::from_value(json!({ "operationId": "abc", "done": false })).unwrap();
        assert_eq!(by_id.id().as_deref(), Some("abc"));
        assert!(!by_id.done);

        let by_path: Operation =
            serde_json::from_value(json!({ "path": "operations/def-123" })).unwrap();
        assert_eq!(by_path.id().as_deref(), Some("def-123"));
        assert!(!by_path.done);

        let neither: Operation = serde_json::from_value(json!({ "done": false })).unwrap();
        assert_eq!(neither.id(), None);
    }

    #[test]
    fn finished_operation_reports_asset_and_version() {
        let operation: Operation = serde_json::from_value(json!({
            "path": "operations/abc",
            "done": true,
            "response": { "assetId": "139145374182770", "revisionId": "7" },
        }))
        .unwrap();

        let response = operation.response.unwrap();
        assert_eq!(response.asset_id, Some(139145374182770));
        assert_eq!(response.revision_id, Some(7));
    }

    #[test]
    fn version_is_optional_and_tolerates_other_shapes() {
        let response = |value: serde_json::Value| -> OperationResponse {
            serde_json::from_value(value).unwrap()
        };

        assert_eq!(response(json!({ "assetId": "5" })).revision_id, None);
        assert_eq!(
            response(json!({ "assetId": 5, "revisionId": 3 })).revision_id,
            Some(3)
        );
        assert_eq!(
            response(json!({ "assetId": "5", "revisionId": "three" })).revision_id,
            None
        );
        assert_eq!(response(json!({ "assetId": 5 })).asset_id, Some(5));
    }

    #[test]
    fn failed_operation_explains_itself() {
        let operation: Operation = serde_json::from_value(json!({
            "path": "operations/abc",
            "done": true,
            "error": { "code": 3, "message": "Asset type cannot be updated" },
        }))
        .unwrap();

        assert_eq!(
            operation.error.unwrap().to_string(),
            "Asset type cannot be updated (code 3)"
        );
    }
}
