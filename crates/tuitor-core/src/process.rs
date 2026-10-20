//! Running a program to ask it something (`node --version`, gdb's DAP server, `docker
//! compose config`) and reading what it says, with a time limit: a program that hangs must
//! not take tuitor with it.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// What a program printed and how it ended.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Answer {
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

/// How a program is asked.
pub struct Ask<'a> {
    pub program: &'a Path,
    pub args: &'a [&'a str],
    pub env: &'a [(&'a str, &'a str)],
    pub cwd: Option<&'a Path>,
    /// Written to its standard input, which is then closed.
    pub input: Option<&'a [u8]>,
    pub timeout: Duration,
}

impl<'a> Ask<'a> {
    pub fn new(program: &'a Path, args: &'a [&'a str]) -> Self {
        Ask { program, args, env: &[], cwd: None, input: None, timeout: Duration::from_secs(10) }
    }
}

/// Runs the program and waits for it, at most `timeout`; one that takes longer is killed.
/// `None` when it cannot be started or did not finish in time.
pub fn ask(ask: &Ask<'_>) -> Option<Answer> {
    let mut command = Command::new(ask.program);
    command.args(ask.args).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.stdin(if ask.input.is_some() { Stdio::piped() } else { Stdio::null() });
    command.envs(ask.env.iter().copied());
    if let Some(cwd) = ask.cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().ok()?;
    if let (Some(input), Some(mut stdin)) = (ask.input, child.stdin.take()) {
        // A program that exits before reading all of it is no reason to give up on its answer.
        let _ = stdin.write_all(input);
    }
    // Read on threads of their own: a full pipe would stop the program before it exits.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let deadline = Instant::now() + ask.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = |reader: Option<thread::JoinHandle<Vec<u8>>>| {
        let bytes = reader.and_then(|r| r.join().ok()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Some(Answer { status, stdout: text(stdout), stderr: text(stderr) })
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_and_gives_up_on_a_program_that_hangs() {
        let sh = Path::new("/bin/sh");
        let answer = ask(&Ask { input: Some(b"hello"), ..Ask::new(sh, &["-c", "cat; echo oops >&2"]) });
        let answer = answer.expect("sh answers");
        assert!(answer.success());
        assert_eq!((answer.stdout.as_str(), answer.stderr.as_str()), ("hello", "oops\n"));

        let slow = Ask { timeout: Duration::from_millis(200), ..Ask::new(sh, &["-c", "sleep 5"]) };
        let started = Instant::now();
        assert!(ask(&slow).is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(ask(&Ask::new(Path::new("/nonexistent/tuitor-program"), &[])).is_none());
    }
}
