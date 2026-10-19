//! Signals that end tuitor the way quitting does, so that it still disconnects its adapters,
//! removes their temporary directories and puts the terminal back: SIGTERM, SIGHUP (its
//! terminal went away) and SIGINT. In raw mode ctrl-c is a key, so SIGINT comes from
//! `kill -INT`, or from ctrl-c in a `--script` run.

use std::io;
use std::process::ExitCode;

#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};

#[cfg(unix)]
pub struct Signals {
    terminate: Signal,
    hangup: Signal,
    interrupt: Signal,
}

#[cfg(unix)]
impl Signals {
    /// From now on these signals no longer end the process at once; [`Signals::recv`] says
    /// when one came. Needs the runtime.
    pub fn new() -> io::Result<Signals> {
        Ok(Signals {
            terminate: signal(SignalKind::terminate())?,
            hangup: signal(SignalKind::hangup())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// Waits for the next one, and returns the exit status it calls for: 128 and its number,
    /// as a shell reports a program that a signal ended.
    pub async fn recv(&mut self) -> ExitCode {
        let kind = tokio::select! {
            _ = self.terminate.recv() => SignalKind::terminate(),
            _ = self.hangup.recv() => SignalKind::hangup(),
            _ = self.interrupt.recv() => SignalKind::interrupt(),
        };
        exit_code(kind.as_raw_value())
    }
}

#[cfg(not(unix))]
pub struct Signals;

#[cfg(not(unix))]
impl Signals {
    pub fn new() -> io::Result<Signals> {
        Ok(Signals)
    }

    /// Ctrl-c, the one signal there is.
    pub async fn recv(&mut self) -> ExitCode {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
        exit_code(2)
    }
}

fn exit_code(signal: i32) -> ExitCode {
    u8::try_from(128 + signal).map_or(ExitCode::FAILURE, ExitCode::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_ends_with_128_and_its_number() {
        assert_eq!(exit_code(15), ExitCode::from(143));
        assert_eq!(exit_code(1), ExitCode::from(129));
        assert_eq!(exit_code(200), ExitCode::FAILURE);
    }
}
