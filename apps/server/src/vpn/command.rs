use std::{io, process::Stdio, time::Duration};

use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

pub async fn output(program: &str, args: &[&str]) -> io::Result<Vec<u8>> {
    let output = timeout(
        Duration::from_secs(5),
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("{program} timed out")))??;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{program} failed with {}",
            output.status
        )));
    }
    Ok(output.stdout)
}

pub async fn run(program: &str, args: &[&str], input: Option<&[u8]>) -> io::Result<()> {
    timeout(Duration::from_secs(5), async {
        let mut child = Command::new(program)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                io::Error::new(error.kind(), format!("could not start {program}: {error}"))
            })?;

        if let Some(input) = input {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("command stdin is unavailable"))?;
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err(io::Error::other(format!("{program} failed with {status}")));
        }

        Ok(())
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("{program} timed out")))?
}
