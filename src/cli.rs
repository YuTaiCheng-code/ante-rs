use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "ante",
    version = crate::VERSION,
    about = "Ghost in the shell",
    long_about = None,
    subcommand_precedence_over_arg = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub run: RunArgs,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RunArgs {
    /// Override the default model name
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// Override the API provider by catalog provider name (default fallback: local)
    #[arg(long)]
    pub provider: Option<String>,

    /// Override the model's effort level for this run (min | low | medium | high | xhigh | max)
    #[arg(long)]
    pub effort: Option<String>,

    /// Shorthand for `--permission-mode yolo`
    #[arg(long)]
    pub yolo: bool,

    /// Run the chat on the alternate screen for this session
    #[arg(long)]
    pub fullscreen: bool,

    /// Override the permission mode for this run (strict | auto | yolo)
    #[arg(long)]
    pub permission_mode: Option<String>,

    /// Override the default system prompt entirely
    #[arg(long)]
    pub system_prompt: Option<String>,

    /// Read the replacement system prompt from a UTF-8 file
    #[arg(long)]
    pub system_prompt_file: Option<String>,

    /// Append content to the system prompt
    #[arg(long)]
    pub append_system_prompt: Option<String>,

    /// Use smaller built-in tool descriptions
    #[arg(long)]
    pub short_prompt: bool,

    /// Load skills for this run, overriding settings
    #[arg(long, action = ArgAction::SetTrue)]
    pub skills: bool,

    /// Do not load skills for this run
    #[arg(long)]
    pub no_skills: bool,

    /// Add these skills to the default set
    #[arg(long, value_delimiter = ',')]
    pub include_skills: Vec<String>,

    /// Exclude these skills
    #[arg(long, value_delimiter = ',')]
    pub exclude_skills: Vec<String>,

    /// Enable auto memory for this run
    #[arg(long)]
    pub enable_auto_memory: bool,

    /// Disable auto memory for this run
    #[arg(long)]
    pub disable_auto_memory: bool,

    /// Use exactly these tools, replacing the default set
    #[arg(long, value_delimiter = ',')]
    pub tools: Option<Vec<String>>,

    /// Add these tools on top of the default set
    #[arg(long, value_delimiter = ',')]
    pub include_tools: Vec<String>,

    /// Exclude these tools
    #[arg(long, value_delimiter = ',', visible_alias = "disallowed-tools")]
    pub exclude_tools: Vec<String>,

    /// Output format for headless mode
    #[arg(long, default_value = "minimal", value_parser = ["json", "human", "minimal"])]
    pub output_format: String,

    /// Prompt to run in headless mode (also supports stdin when piped)
    #[arg(short = 'p', long)]
    pub prompt: Option<String>,

    /// Start the TUI with this text in the composer, typed but not sent
    #[arg(long)]
    pub draft: Option<String>,

    /// Run a verification check after the task completes (headless mode only)
    #[arg(long)]
    pub check: bool,

    /// Resume a previous session by ID; without an ID, pick one interactively
    #[arg(short = 'r', long, num_args = 0..=1, default_missing_value = "")]
    pub resume: Option<String>,

    /// Save session transcripts and resumable snapshots, overriding settings
    #[arg(long)]
    pub session_save: bool,

    /// Do not save session transcripts or resumable snapshots
    #[arg(long)]
    pub no_session_save: bool,

    /// Run on a local model through the local inference engine
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub offline_model: Option<String>,

    /// Use a named settings profile
    #[arg(long, env = "ANTE_PROFILE", global = true)]
    pub profile: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage credentials: OAuth sign-ins and stored secrets
    Auth(AuthCmd),
    /// Show subscription usage limits for stored OAuth sign-ins
    Usage(UsageCmd),
    /// Run as a long-lived server
    Serve(ServeCmd),
    /// Update Ante: install the channel's newest release, or install an exact version
    Update(UpdateCmd),
    /// Manage the local inference engine used by offline mode
    Offline(OfflineCmd),
    /// Collect a redacted bug-report bundle (logs + environment) to share with the Ante team
    Rage(RageCmd),
    /// Print the loaded provider catalog as JSON
    Catalog,
    /// Boot-time self-check: validate the runtime and report startup phase timings
    Doctor(DoctorCmd),
}

#[derive(Args, Debug)]
pub struct AuthCmd {
    #[command(subcommand)]
    pub command: AuthSub,
}

#[derive(Subcommand, Debug)]
pub enum AuthSub {
    /// List stored credentials
    List,
    /// Sign in with an OAuth profile, or to the connector that uses one
    Login(AuthLogin),
    /// Forget a sign-in; the same as `rm oauth:NAME`
    Logout(AuthName),
    /// Store a secret (an API key) for a connector, read from stdin
    Add(AuthName),
    /// Forget a stored credential
    Rm(AuthName),
}

#[derive(Args, Debug)]
pub struct AuthLogin {
    /// An OAuth profile, a provider, or an MCP server; optionally with a `/label`
    pub name: String,
    /// Print the authorization URL instead of opening a browser
    #[arg(long)]
    pub no_browser: bool,
    /// Override the localhost callback port
    #[arg(long)]
    pub callback_port: Option<u16>,
}

#[derive(Args, Debug)]
pub struct AuthName {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct UsageCmd {
    /// Exact stored sign-in name, such as `openai` or `openai/work`
    pub name: Option<String>,
    /// Print account status and usage windows as JSON, with UTC reset timestamps
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ServeCmd {
    /// Serve the JSONL protocol over stdin/stdout (optional, default transport today)
    #[arg(long)]
    pub stdio: bool,
    /// Serve the protocol over websocket on the given loopback socket address
    #[arg(long, value_name = "ADDR")]
    pub ws: Option<String>,
    /// Serve the JSONL protocol on a Unix domain socket file
    #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "")]
    pub sock: Option<String>,
    /// Serve a local model through the local inference engine
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub offline_model: Option<String>,
}

#[derive(Args, Debug)]
pub struct UpdateCmd {
    /// Release channel to update from (for example: stable, nightly)
    pub channel: Option<String>,
    /// Release channel to update from
    #[arg(long)]
    pub channel_flag: Option<String>,
    /// Install an exact version instead of the channel's newest release
    #[arg(long)]
    pub version: Option<String>,
}

#[derive(Args, Debug)]
pub struct OfflineCmd {
    #[command(subcommand)]
    pub command: OfflineSub,
}

#[derive(Subcommand, Debug)]
pub enum OfflineSub {
    /// Install the local inference engine (Ante's pinned llama.cpp build)
    Install,
    /// Report the engine, model directory, default model, Ante's router, and running local servers
    Status(OfflineStatus),
    /// List the models `--offline-model` can start
    List(OfflineList),
    /// Set the model a bare `--offline-model` starts, and how it loads
    Use(OfflineUse),
    /// Stop the home's llama.cpp router, kept by an earlier launch or in use
    Stop,
}

#[derive(Args, Debug)]
pub struct OfflineStatus {
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct OfflineList {
    /// Print the list as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct OfflineUse {
    /// A GGUF path, a file in the model directory, or a verified model name or URL
    pub model: String,
    #[arg(long)]
    pub context: Option<u64>,
    #[arg(long)]
    pub tokens: Option<u64>,
    #[arg(long, value_parser = ["on", "off"])]
    pub thinking: Option<String>,
    #[arg(long)]
    pub temperature: Option<String>,
    #[arg(long)]
    pub extra: Option<String>,
}

#[derive(Args, Debug)]
pub struct RageCmd {
    /// Write to this path instead of `./ante-rage-<timestamp>.tar.gz`
    #[arg(long, value_name = "PATH")]
    pub output: Option<String>,
    /// Write an uncompressed report directory instead of a `.tar.gz` file
    #[arg(long)]
    pub no_compress: bool,
    /// Also include DEBUG/TRACE log lines
    #[arg(long)]
    pub verbose_logs: bool,
    /// Collect every log modified within this window
    #[arg(long, value_name = "DURATION")]
    pub since: Option<String>,
}

#[derive(Args, Debug)]
pub struct DoctorCmd {
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}
