use std::{fs, io::Write, path::PathBuf};

use biome_term_client::{BiomeTermClient, BiomeTermClientBuilder, CreatePaneOptions};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;

#[derive(Parser)]
#[command(name = "biome-term", about = "CLI for the biome_term PTY server")]
struct Cli {
    /// Server base URL
    #[arg(long)]
    url: Option<String>,

    /// API key sent as Authorization: Bearer <key>
    #[arg(long, env = "BIOME_API_KEY")]
    api_key: Option<String>,

    /// PEM file containing one or more trusted root certificates for HTTPS/WSS
    #[arg(long, env = "BIOME_TLS_CA_CERT")]
    ca_cert: Option<PathBuf>,

    /// Disable TLS certificate and hostname validation
    #[arg(long, action = clap::ArgAction::SetTrue)]
    insecure: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new pane
    Create {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "80")]
        cols: u16,
        #[arg(long, default_value = "24")]
        rows: u16,
        #[arg(long)]
        shell: Option<String>,
    },
    /// List all panes
    List,
    /// Delete a pane
    Delete { id: String },
    /// Send text input to a pane
    Input {
        id: String,
        /// Text to send (tip: append \\n to submit a command)
        text: String,
    },
    /// Resize a pane
    Resize { id: String, cols: u16, rows: u16 },
    /// Print the current screen of a pane
    Screen { id: String },
    /// Dump event log for a pane (raw PTY bytes written to stdout)
    Events {
        id: String,
        /// Only return events after this sequence number
        #[arg(long)]
        after: Option<u64>,
    },
    /// Stream live PTY output from a pane (Ctrl+C to stop)
    Stream { id: String },
    /// Stream pane lifecycle events as JSON (Ctrl+C to stop)
    Lifecycle,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = match build_client(&cli) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let result = run(cli.cmd, client).await;
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn build_client(cli: &Cli) -> Result<BiomeTermClient, biome_term_client::Error> {
    let url = cli
        .url
        .clone()
        .or_else(default_url_from_env)
        .unwrap_or_else(|| "http://localhost:3021".to_string());

    let mut builder = BiomeTermClient::builder(&url);
    if let Some(api_key) = cli.api_key.clone() {
        builder = builder.api_key(api_key);
    }
    if let Some(path) = &cli.ca_cert {
        builder = add_root_cert_from_path(builder, path)?;
    }
    if cli.insecure || env_flag("BIOME_TLS_INSECURE") {
        builder = builder
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true);
    }
    builder.build()
}

fn add_root_cert_from_path(
    builder: BiomeTermClientBuilder,
    path: &PathBuf,
) -> Result<BiomeTermClientBuilder, biome_term_client::Error> {
    Ok(builder.add_root_certificate_pem(fs::read(path)?))
}

fn default_url_from_env() -> Option<String> {
    env_string("BIOME_TERM_URL").or_else(|| env_string("BIOME_URL"))
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_flag(name: &str) -> bool {
    env_string(name).is_some_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

async fn run(cmd: Cmd, client: BiomeTermClient) -> Result<(), biome_term_client::Error> {
    match cmd {
        Cmd::Create {
            name,
            cols,
            rows,
            shell,
        } => {
            let opts = CreatePaneOptions {
                cols: Some(cols),
                rows: Some(rows),
                shell,
                name,
            };
            let pane = client.create_pane(opts).await?;
            println!("{}", serde_json::to_string_pretty(&pane).unwrap());
        }

        Cmd::List => {
            let panes = client.list_panes().await?;
            if panes.is_empty() {
                println!("(no panes)");
                return Ok(());
            }
            println!(
                "{:<38} {:<20} {:>4} {:>4}  STATUS",
                "ID", "NAME", "COLS", "ROWS"
            );
            println!("{}", "-".repeat(75));
            for p in &panes {
                let name = p.name.as_deref().unwrap_or("-");
                let status = if p.terminated {
                    "terminated"
                } else {
                    "running"
                };
                println!(
                    "{:<38} {:<20} {:>4} {:>4}  {}",
                    p.id, name, p.cols, p.rows, status
                );
            }
        }

        Cmd::Delete { id } => {
            client.delete_pane(&id).await?;
            println!("deleted");
        }

        Cmd::Input { id, text } => {
            client.send_input(&id, text.as_bytes()).await?;
        }

        Cmd::Resize { id, cols, rows } => {
            client.resize_pane(&id, cols, rows).await?;
            println!("resized to {cols}x{rows}");
        }

        Cmd::Screen { id } => {
            let screen = client.get_screen(&id).await?;
            for row in &screen.rows {
                println!("{row}");
            }
        }

        Cmd::Events { id, after } => {
            let events = client.get_events(&id, after).await?;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            for event in &events {
                out.write_all(&event.data).unwrap();
            }
            out.flush().unwrap();
        }

        Cmd::Stream { id } => {
            let mut stream = client.stream_pane(&id).await?;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            while let Some(result) = stream.next().await {
                let event = result?;
                out.write_all(&event.data).unwrap();
                out.flush().unwrap();
            }
        }

        Cmd::Lifecycle => {
            let mut stream = client.stream_lifecycle().await?;
            while let Some(result) = stream.next().await {
                let event = result?;
                println!("{}", serde_json::to_string(&event).unwrap());
            }
        }
    }
    Ok(())
}
