use std::io::Write;

use biome_term_client::{BiomeTermClient, CreatePaneOptions};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;

#[derive(Parser)]
#[command(name = "biome-term", about = "CLI for the biome_term PTY server")]
struct Cli {
    /// Server base URL
    #[arg(long, env = "BIOME_TERM_URL", default_value = "http://localhost:3000")]
    url: String,

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
    let client = BiomeTermClient::new(&cli.url);

    let result = run(cli.cmd, client).await;
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cmd: Cmd, client: BiomeTermClient) -> Result<(), biome_term_client::Error> {
    match cmd {
        Cmd::Create { name, cols, rows, shell } => {
            let opts = CreatePaneOptions { cols: Some(cols), rows: Some(rows), shell, name };
            let pane = client.create_pane(opts).await?;
            println!("{}", serde_json::to_string_pretty(&pane).unwrap());
        }

        Cmd::List => {
            let panes = client.list_panes().await?;
            if panes.is_empty() {
                println!("(no panes)");
                return Ok(());
            }
            println!("{:<38} {:<20} {:>4} {:>4}  STATUS", "ID", "NAME", "COLS", "ROWS");
            println!("{}", "-".repeat(75));
            for p in &panes {
                let name = p.name.as_deref().unwrap_or("-");
                let status = if p.terminated { "terminated" } else { "running" };
                println!("{:<38} {:<20} {:>4} {:>4}  {}", p.id, name, p.cols, p.rows, status);
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
