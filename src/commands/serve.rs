use tokio::io::{AsyncBufReadExt, BufReader};

use crate::cli::ServeCmd;
use crate::settings::Settings;

pub async fn run(cmd: ServeCmd, settings: Settings) -> i32 {
    if let Some(path) = cmd.sock {
        let path = if path.is_empty() {
            crate::util::ante_home().join("run/serve.sock")
        } else {
            crate::util::path_from_str(&path)
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(&path);
        let listener = match tokio::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("failed to bind {}: {e}", path.display());
                return 1;
            }
        };
        println!("listening on {}", path.display());
        if let Ok((stream, _)) = listener.accept().await {
            let mut reader = BufReader::new(stream);
            let mut server = crate::protocol::Server::new(settings);
            let mut line = String::new();
            while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                let keep = server.handle_line(&line).await;
                line.clear();
                if !keep {
                    break;
                }
            }
        }
        return 0;
    }
    if let Some(addr) = cmd.ws {
        eprintln!("websocket transport ({addr}) is not implemented in this build; use --stdio or --sock");
        return 1;
    }

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut server = crate::protocol::Server::new(settings);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                if !server.handle_line(&line).await {
                    break;
                }
            }
            Err(e) => {
                eprintln!("stdin closed: {e}");
                break;
            }
        }
    }
    0
}
