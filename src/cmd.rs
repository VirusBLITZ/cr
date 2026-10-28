use std::io::{ErrorKind, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Runs `cmd`, optionally feeding `stdin`, and returns its stdout.
/// Fails with the command's stderr if it exits non-zero.
pub fn run(cmd: &mut Command, stdin: Option<&str>) -> Result<String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        ErrorKind::NotFound => anyhow::anyhow!("`{program}` not found on PATH"),
        _ => anyhow::Error::new(e).context(format!("failed to start `{program}`")),
    })?;

    // Write stdin from a separate thread so a chatty child can't deadlock us.
    let writer = stdin.map(|input| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let input = input.to_owned();
        std::thread::spawn(move || pipe.write_all(input.as_bytes()))
    });

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to wait for `{program}`"))?;
    if let Some(writer) = writer {
        writer.join().expect("stdin writer panicked").ok();
    }

    if !output.status.success() {
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
        bail!(
            "`{program} {}` failed ({}):\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
