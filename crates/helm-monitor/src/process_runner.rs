//! Bounded subprocess execution with process-group cleanup on every exit path.

use std::{
    io,
    process::{Output, Stdio},
    time::Duration,
};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::watch,
};

#[derive(Debug, Clone, Copy)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub output_bytes_per_stream: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            output_bytes_per_stream: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CancellationToken(watch::Sender<bool>);

impl Default for CancellationToken {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}\nstdout: {stdout}\nstderr: {stderr}")]
pub struct ProcessFailure {
    pub message: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

struct ProcessGroup(Pid);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = killpg(self.0, Signal::SIGKILL);
    }
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    buffer: &mut Vec<u8>,
    limit: usize,
) -> io::Result<()> {
    let mut chunk = [0u8; 4096];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        let remaining = limit.saturating_sub(buffer.len());
        buffer.extend_from_slice(&chunk[..read.min(remaining)]);
        if read > remaining {
            return Err(io::Error::other("process output limit exceeded"));
        }
    }
}

pub async fn run_command(
    command: Command,
    limits: ProcessLimits,
    cancel: &CancellationToken,
) -> Result<Output, ProcessFailure> {
    // Collectors compose many commands; keep their async state off the caller's stack.
    Box::pin(run_command_inner(command, limits, cancel)).await
}

async fn run_command_inner(
    mut command: Command,
    limits: ProcessLimits,
    cancel: &CancellationToken,
) -> Result<Output, ProcessFailure> {
    let failure = |message: String, stdout: &[u8], stderr: &[u8], timed_out| ProcessFailure {
        message,
        stdout: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
        timed_out,
    };
    if limits.timeout.is_zero() {
        return Err(failure(
            "execution deadline expired before launch".into(),
            &[],
            &[],
            true,
        ));
    }
    if cancel.is_cancelled() {
        return Err(failure(
            "execution cancelled before launch".into(),
            &[],
            &[],
            false,
        ));
    }
    command
        .process_group(0)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| failure(format!("cannot spawn process: {e}"), &[], &[], false))?;
    let group = ProcessGroup(Pid::from_raw(
        child.id().expect("spawned child has a pid") as i32
    ));
    let stdout_pipe = child.stdout.take().expect("stdout is piped");
    let stderr_pipe = child.stderr.take().expect("stderr is piped");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut timed_out = false;
    let result = {
        let collect = async {
            let (status, (), ()) = tokio::try_join!(
                child.wait(),
                read_bounded(stdout_pipe, &mut stdout, limits.output_bytes_per_stream),
                read_bounded(stderr_pipe, &mut stderr, limits.output_bytes_per_stream),
            )?;
            Ok(status)
        };
        tokio::select! {
            result = collect => result,
            _ = cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "execution cancelled")),
            _ = tokio::time::sleep(limits.timeout) => {
                timed_out = true;
                Err(io::Error::new(io::ErrorKind::TimedOut, "execution timed out"))
            }
        }
    };
    // Also kill background descendants after their parent exits successfully.
    drop(group);
    match result {
        Ok(status) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        Err(error) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            Err(failure(error.to_string(), &stdout, &stderr, timed_out))
        }
    }
}

pub async fn run_program(
    program: &str,
    args: &[impl AsRef<std::ffi::OsStr>],
    timeout: Duration,
) -> Result<Output, ProcessFailure> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    run_command(
        command,
        ProcessLimits {
            timeout,
            ..Default::default()
        },
        &Default::default(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn runner_future_does_not_inflate_collector_stacks() {
        let token = CancellationToken::default();
        let future = run_command(shell("true"), ProcessLimits::default(), &token);
        let bytes = std::mem::size_of_val(&future);
        assert!(
            bytes < 16 * 1024,
            "runner future requires {bytes} stack bytes"
        );
    }

    #[tokio::test]
    async fn timeout_preserves_partial_diagnostics() {
        let error = run_command(
            shell("printf partial; printf problem >&2; sleep 5"),
            ProcessLimits {
                timeout: Duration::from_millis(100),
                ..ProcessLimits::default()
            },
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
        assert!(error.timed_out);
        assert_eq!(error.stdout, "partial");
        assert_eq!(error.stderr, "problem");
    }

    #[tokio::test]
    async fn output_is_bounded_and_process_stops() {
        let error = run_command(
            shell("while :; do printf 1234567890; done"),
            ProcessLimits {
                output_bytes_per_stream: 100,
                timeout: Duration::from_secs(1),
            },
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
        assert!(error.message.contains("output limit"));
        assert_eq!(error.stdout.len(), 100);
    }

    #[tokio::test]
    async fn cancellation_before_launch_has_no_side_effect() {
        let token = CancellationToken::default();
        token.cancel();
        let error = run_command(shell("exit 42"), ProcessLimits::default(), &token)
            .await
            .unwrap_err();
        assert!(error.message.contains("before launch"));
    }

    #[tokio::test]
    async fn explicit_cancellation_preserves_partial_output() {
        let token = CancellationToken::default();
        let trigger = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            trigger.cancel();
        });
        let error = run_command(
            shell("printf partial; sleep 5"),
            ProcessLimits::default(),
            &token,
        )
        .await
        .unwrap_err();
        assert!(error.message.contains("cancelled"));
        assert_eq!(error.stdout, "partial");
    }
}
