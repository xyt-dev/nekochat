use std::{path::{Path, PathBuf}, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use nekochat_search::config::Config as SearchConfig;
use nekochat_server::config::Config;
use tokio::process::{Child, Command};
use tokio::io::AsyncReadExt;

pub async fn serve(config: Config, search_config: &Path, program: Option<PathBuf>) -> Result<()> {
    let search = SearchConfig::load(search_config).map_err(anyhow::Error::msg)?;
    ensure!(search.listen.ip().is_loopback() && search.listen.port() != 0,
        "managed search requires a fixed loopback listen address");
    let program = match program {
        Some(program) => program,
        None => std::env::current_exe()?.parent().context("cannot locate executable directory")?
            .join(if cfg!(windows) { "nekochat-search.exe" } else { "nekochat-search" }),
    };
    // Report existing listeners early; ownership is proved by the child's private pipe.
    drop(tokio::net::TcpListener::bind(search.listen).await
        .context("local search listen address is already in use")?);

    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let shutdown = async {
        #[cfg(unix)]
        tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await?;
        Ok::<(), anyhow::Error>(())
    };
    tokio::pin!(shutdown);

    let mut command = Command::new(program);
    command.arg("--config").arg(search_config).arg("--notify-ready")
        .stdin(Stdio::null()).stdout(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("cannot start local search; build both nekochat-server and nekochat-search")?;
    let ready = tokio::select! {
        result = wait_ready(&mut child, search.listen) => result,
        result = &mut shutdown => { stop(&mut child).await?; return result; }
    };
    if let Err(error) = ready {
        stop(&mut child).await?;
        return Err(error);
    }
    tracing::info!(addr = %search.listen, "local search ready; starting web");
    let result = tokio::select! {
        result = nekochat_server::serve(config) => result,
        status = child.wait() => match status {
            Ok(status) => Err(anyhow::anyhow!("local search exited while web was running ({status})")),
            Err(error) => Err(error).context("cannot observe local search process"),
        },
        result = &mut shutdown => result,
    };
    let cleanup = stop(&mut child).await;
    result.and(cleanup)
}

async fn wait_ready(child: &mut Child, address: std::net::SocketAddr) -> Result<()> {
    let wait = async {
        let expected = format!("nekochat-search-ready {address}\n");
        let mut received = vec![0; expected.len()];
        child.stdout.as_mut().context("local search readiness pipe is missing")?
            .read_exact(&mut received).await.context("local search exited before readiness")?;
        ensure!(received == expected.as_bytes(), "invalid local search readiness notification");
        if let Some(status) = child.try_wait()? { bail!("local search exited before readiness ({status})"); }
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(20), wait).await.context("local search readiness timed out")?
}

async fn stop(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_some() { return Ok(()); }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The PID belongs to this unreaped child, so it cannot have been reused.
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                child.kill().await.context("cannot stop local search")?;
                return Ok(());
            }
        }
    }
    #[cfg(not(unix))]
    child.start_kill()?;
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(status) => { status.context("cannot reap local search")?; }
        Err(_) => child.kill().await.context("cannot stop local search after shutdown timeout")?,
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::{stop, wait_ready};
    use std::{os::unix::process::ExitStatusExt, process::Stdio};
    use tokio::process::Command;

    #[tokio::test]
    async fn stopping_owned_child_terminates_gracefully_and_reaps_it() {
        let mut child = Command::new("sleep").arg("60").kill_on_drop(true).spawn().unwrap();
        stop(&mut child).await.unwrap();
        assert_eq!(child.try_wait().unwrap().unwrap().signal(), Some(libc::SIGTERM));
        assert!(child.id().is_none());
        stop(&mut child).await.unwrap();
    }

    #[tokio::test]
    async fn stopping_already_exited_child_preserves_its_exit_status() {
        let mut child = Command::new("sh").args(["-c", "exit 7"]).kill_on_drop(true).spawn().unwrap();
        assert_eq!(child.wait().await.unwrap().code(), Some(7));
        stop(&mut child).await.unwrap();
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(7));
    }

    #[tokio::test]
    async fn readiness_rejects_a_child_notification_for_a_different_listener() {
        let mut child = Command::new("sh")
            .args(["-c", "printf 'nekochat-search-ready 127.0.0.1:2222\\n'; exec sleep 60"])
            .stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let ready = wait_ready(&mut child, "127.0.0.1:1111".parse().unwrap()).await;
        stop(&mut child).await.unwrap();
        assert!(ready.is_err());
    }

    #[tokio::test]
    async fn readiness_rejects_an_exited_child_without_a_notification() {
        let mut child = Command::new("sh").args(["-c", "exit 7"])
            .stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let ready = wait_ready(&mut child, "127.0.0.1:1111".parse().unwrap()).await;
        stop(&mut child).await.unwrap();
        assert!(ready.is_err());
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(7));
    }
}
