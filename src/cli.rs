use crate::config::CreatorType;
use clap::{Args, Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Upload and reference Roblox assets in code.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[command(flatten)]
    pub verbose: Verbosity<InfoLevel>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Sync assets.
    Sync(SyncArgs),

    /// Uploads a single asset and returns the asset ID.
    Upload(UploadArgs),

    /// Migrates a lockfile to the latest version.
    ///
    /// You can only run this once per upgrade, and it will overwrite the existing lockfile.
    /// Keep in mind that because pre-1.0 did not support multiple inputs, you'll need to provide a default input name for that migration.
    /// The pre-1.0 migration entails hashing your files again and updating the lockfile with the new hashes.
    /// We basically pretend nothing has changed, so your assets don't get reuploaded.
    MigrateLockfile(MigrateLockfileArgs),

    #[command(hide = true)]
    GenerateConfigSchema,
}

#[derive(Subcommand, Clone, Copy)]
pub enum SyncTarget {
    /// Upload assets to Roblox cloud.
    Cloud {
        /// Error if assets would be uploaded.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write assets to the Roblox Studio content folder.
    Studio {
        /// Watch for file changes and re-sync.
        #[arg(long)]
        watch: bool,
    },
    /// Write assets to the .asphalt-debug folder.
    Debug,
}

impl SyncTarget {
    pub fn write_on_sync(&self) -> bool {
        matches!(self, SyncTarget::Cloud { dry_run: false })
    }

    pub fn is_watch(&self) -> bool {
        matches!(self, SyncTarget::Studio { watch: true })
    }
}

#[derive(Args, Clone)]
pub struct SyncArgs {
    /// Your Open Cloud API key.
    #[arg(short, long, env = "ASPHALT_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,

    /// Where Asphalt should sync assets to.
    #[command(subcommand)]
    target: Option<SyncTarget>,

    /// Provides Roblox with the amount of Robux that you are willing to spend on each non-free asset upload.
    #[arg(long)]
    pub expected_price: Option<u32>,

    /// Path to the project directory. Defaults to the current directory.
    #[arg(short, long, default_value = ".")]
    pub project: PathBuf,
}

impl SyncArgs {
    pub fn target(&self) -> SyncTarget {
        self.target.unwrap_or(SyncTarget::Cloud { dry_run: false })
    }
}

#[derive(Args)]
pub struct UploadArgs {
    /// The file to upload.
    pub path: String,

    /// Upload the file as a new version of this existing asset instead of creating a new asset.
    ///
    /// The asset keeps its ID, and the result is printed as JSON with the new version number
    /// (see --json). Roblox decides which asset types can be updated this way.
    #[arg(long)]
    pub asset_id: Option<u64>,

    /// The creator type of the asset. Required unless --asset-id is given.
    #[arg(long, required_unless_present = "asset_id", requires = "creator_id")]
    pub creator_type: Option<CreatorType>,

    /// The creator ID of the asset. Required unless --asset-id is given.
    #[arg(long, required_unless_present = "asset_id", requires = "creator_type")]
    pub creator_id: Option<u64>,

    /// Your Open Cloud API key.
    #[arg(short, long, env = "ASPHALT_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,

    /// Whether to alpha bleed if it's an image.
    #[arg(long, default_value = "true")]
    pub bleed: bool,

    /// Format the response as a link.
    #[arg(long, conflicts_with = "json")]
    pub link: bool,

    /// Print the asset ID and its version number as JSON: {"asset_id":123,"version":4}.
    ///
    /// This is the default with --asset-id. The version is null when Roblox does not report one.
    #[arg(long)]
    pub json: bool,

    /// Provides Roblox with the amount of Robux that you are willing to spend on each non-free asset upload.
    #[arg(long, requires_all = ["creator_type", "creator_id"])]
    pub expected_price: Option<u32>,
}

#[derive(Args)]
pub struct MigrateLockfileArgs {
    /// The default input name to use. Only applies when upgrading from V0 to V1.
    pub input_name: Option<String>,

    /// Path to the project directory. Defaults to the current directory.
    #[arg(short, long, default_value = ".")]
    pub project: PathBuf,
}
