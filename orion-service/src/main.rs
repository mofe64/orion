use orion_service::{Host, Request, rpc};
use std::{path::PathBuf, sync::Arc};

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Orion service runtime");
    let result = runtime.block_on(run());
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    if let Err(error) = result {
        eprintln!("orion-service: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let command = arguments.first().map(String::as_str).unwrap_or("serve");
    if matches!(command, "--help" | "help") {
        println!(
            "Usage: orion-service [serve [--no-autostart] | status | check]\nPi installation: python3 scripts/install_pi_voice_stack.py"
        );
        return Ok(());
    }
    if arguments.len() > 2
        || (arguments.len() == 2 && !(command == "serve" && arguments[1] == "--no-autostart"))
    {
        return Err("Unexpected arguments; use --help".into());
    }
    let directory = rpc::service_home()?;
    if command == "status" {
        println!("{}", rpc::call(&directory, &Request::Status)?);
        return Ok(());
    }
    if command == "check" {
        let root = orion_service::project_root()?;
        let python = std::env::var_os("ORION_STUDIO_VOICE_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("speech/.venv/bin/python"));
        if !python.is_file() || !root.join("speech/orion_speech_worker/worker.py").is_file() {
            return Err(
                "Prepare the speech worker and its Python environment before starting".into(),
            );
        }
        orion_service::settings::load_voice_settings()?.validate()?;
        println!("Orion service files and settings are ready");
        return Ok(());
    }
    if command != "serve" {
        return Err("Unknown command; use --help".into());
    }
    let host = Arc::new(Host::new(&directory)?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let publication = rpc::Publication::new(
        &directory,
        &rpc::Connection {
            protocol: rpc::PROTOCOL,
            address: listener.local_addr().map_err(|e| e.to_string())?,
            token: token.clone(),
        },
    )?;
    let autostart = !arguments.iter().any(|arg| arg == "--no-autostart");
    let mut tasks = tokio::task::JoinSet::new();
    let mut maintenance: Option<tokio::task::JoinHandle<()>> = None;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| e.to_string())?;
    let stop = async {
        #[cfg(unix)]
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
    };
    tokio::pin!(stop);
    eprintln!("orion-service: control service ready");
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = interval.tick(), if autostart => {
                if maintenance.as_ref().is_none_or(|task| task.is_finished()) {
                    let host = host.clone();
                    maintenance = Some(tokio::spawn(async move { host.maintain().await; }));
                }
            },
            Some(_) = tasks.join_next() => {},
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|e| e.to_string())?;
                if tasks.len() < 16 {
                    let token = token.clone(); let host = host.clone();
                    tasks.spawn(async move { rpc::serve(stream, &token, host).await; });
                }
            }
        }
    }
    drop(publication);
    if let Some(task) = maintenance {
        task.abort();
        let _ = task.await;
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    host.shutdown();
    eprintln!("orion-service: stopped");
    Ok(())
}
