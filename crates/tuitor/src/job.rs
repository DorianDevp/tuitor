//! The editor `edit` runs in tuitor's terminal, run as a shell runs a job: in a process group
//! of its own that is given the terminal, so that ctrl-z in the editor stops the editor
//! alone and the terminal comes back to tuitor. tuitor keeps it stopped, and `edit` brings
//! it back as it was left, with what it has open. And ctrl-z in tuitor, which stops tuitor
//! as a job of the shell that started it.

use crate::editor::Editor;

/// How an editor's turn at the terminal ended.
pub enum Turn {
    /// It stopped, ctrl-z in it: it goes on when it is resumed.
    Stopped(Job),
    /// It exited, or could not run.
    Ended(Result<String, String>),
}

#[cfg(unix)]
pub use unix::{Job, can_suspend, suspend_self};

#[cfg(unix)]
mod unix {
    use std::os::fd::AsFd;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    use nix::errno::Errno;
    use nix::sys::signal::{SigSet, SigmaskHow, Signal, killpg, pthread_sigmask};
    use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
    use nix::unistd::{Pid, getpgrp, getpid, getppid, getsid, tcsetpgrp};

    use super::{Editor, Turn};

    /// An editor running in a process group of its own, which is its pid.
    pub struct Job {
        pid: Pid,
        name: String,
        place: String,
    }

    impl Job {
        /// Starts `editor` in a process group of its own and gives it the terminal; the caller
        /// has put the terminal back as the shell had it.
        pub fn start(editor: &Editor) -> Result<Job, String> {
            let child = Command::new(&editor.program)
                .args(&editor.args)
                .process_group(0)
                .spawn()
                .map_err(|e| format!("cannot run {}: {e}", editor.program))?;
            let pid = Pid::from_raw(i32::try_from(child.id()).map_err(|e| e.to_string())?);
            // Dropping the Child neither waits for it nor ends it; `turn` waits for it.
            drop(child);
            let job = Job { pid, name: editor.name(), place: editor.place.clone() };
            job.give_terminal();
            Ok(job)
        }

        /// Gives a stopped job the terminal and lets it go on, as `fg` does.
        pub fn resume(&self) {
            self.give_terminal();
            if let Err(err) = killpg(self.pid, Signal::SIGCONT) {
                tracing::warn!("continuing {}: {err}", self.name);
            }
        }

        /// The program's name, without its directory.
        pub fn name(&self) -> &str {
            &self.name
        }

        /// Waits until the job stops or exits, then takes the terminal back for tuitor.
        pub async fn turn(self) -> Turn {
            let name = self.name.clone();
            tokio::task::spawn_blocking(move || self.wait())
                .await
                .unwrap_or_else(|err| Turn::Ended(Err(format!("{name}: {err}"))))
        }

        fn wait(self) -> Turn {
            loop {
                match waitpid(self.pid, Some(WaitPidFlag::WUNTRACED)) {
                    // It read or set the terminal before it was given it: give it again.
                    Ok(WaitStatus::Stopped(_, Signal::SIGTTIN | Signal::SIGTTOU)) => self.resume(),
                    Ok(WaitStatus::Stopped(..)) => {
                        take_terminal();
                        return Turn::Stopped(self);
                    }
                    Ok(WaitStatus::Exited(_, code)) => {
                        take_terminal();
                        return Turn::Ended(if code == 0 {
                            Ok(format!("back from {} ({})", self.name, self.place))
                        } else {
                            Err(format!("{} ended with exit status {code}", self.name))
                        });
                    }
                    Ok(WaitStatus::Signaled(_, signal, _)) => {
                        take_terminal();
                        return Turn::Ended(Err(format!("{} ended by {signal}", self.name)));
                    }
                    Ok(_) | Err(Errno::EINTR) => {}
                    Err(err) => {
                        take_terminal();
                        return Turn::Ended(Err(format!("waiting for {}: {err}", self.name)));
                    }
                }
            }
        }

        /// Ends a stopped job as the terminal's going away would: a hangup, then the signal to
        /// go on so that it gets it. Neovim and Vim keep a swap file of what was not saved. It
        /// has the terminal meanwhile, since one that went on in the background would stop
        /// again as it sets the terminal up, and tuitor waits a moment for it to end, so that
        /// what it says as it ends is on tuitor's screen, which goes away, not the shell's.
        pub fn hang_up(self) {
            self.give_terminal();
            for signal in [Signal::SIGHUP, Signal::SIGCONT] {
                if let Err(err) = killpg(self.pid, signal) {
                    tracing::warn!("{signal} to {}: {err}", self.name);
                }
            }
            let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < until {
                match waitpid(self.pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => std::thread::sleep(std::time::Duration::from_millis(20)),
                    _ => break,
                }
            }
            take_terminal();
        }

        fn give_terminal(&self) {
            if let Err(err) = tcsetpgrp(std::io::stdin().as_fd(), self.pid) {
                tracing::warn!("giving the terminal to {}: {err}", self.name);
            }
        }
    }

    /// Makes tuitor's process group the terminal's foreground one again. tuitor is in the
    /// background then, and the kernel stops a process of the background that sets the
    /// terminal's group with SIGTTOU unless it blocks the signal, as this thread does meanwhile.
    fn take_terminal() {
        let mut ttou = SigSet::empty();
        ttou.add(Signal::SIGTTOU);
        let mut before = SigSet::empty();
        if let Err(err) = pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&ttou), Some(&mut before)) {
            tracing::warn!("blocking SIGTTOU: {err}");
        }
        if let Err(err) = tcsetpgrp(std::io::stdin().as_fd(), getpgrp()) {
            tracing::warn!("taking the terminal back: {err}");
        }
        if let Err(err) = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&before), None) {
            tracing::warn!("restoring the signal mask: {err}");
        }
    }

    /// Whether a shell can bring tuitor back once it stops. The kernel does not stop a process
    /// group that no shell watches (one that leads its session, as a program a terminal or
    /// Neovim's `:terminal` starts with no shell does, or whose parent is in another
    /// session), and nothing would bring it back.
    pub fn can_suspend() -> bool {
        let Ok(session) = getsid(None) else { return false };
        session != getpid() && getsid(Some(getppid())) == Ok(session)
    }

    /// Stops tuitor's process group, as ctrl-z in a shell does, until the shell continues it.
    pub fn suspend_self() -> Result<(), String> {
        killpg(getpgrp(), Signal::SIGTSTP).map_err(|err| format!("stopping tuitor: {err}"))
    }
}

/// Without job control the editor runs until it exits, and tuitor does not stop.
#[cfg(not(unix))]
pub use other::{Job, can_suspend, suspend_self};

#[cfg(not(unix))]
mod other {
    use super::{Editor, Turn};

    pub struct Job {
        program: String,
        args: Vec<String>,
        name: String,
        place: String,
    }

    impl Job {
        pub fn start(editor: &Editor) -> Result<Job, String> {
            Ok(Job {
                program: editor.program.clone(),
                args: editor.args.clone(),
                name: editor.name(),
                place: editor.place.clone(),
            })
        }

        pub fn resume(&self) {}

        pub fn name(&self) -> &str {
            &self.name
        }

        pub async fn turn(self) -> Turn {
            let status = tokio::process::Command::new(&self.program).args(&self.args).status().await;
            Turn::Ended(match status {
                Ok(status) if status.success() => Ok(format!("back from {} ({})", self.name, self.place)),
                Ok(status) => Err(format!("{} ended with {status}", self.name)),
                Err(err) => Err(format!("cannot run {}: {err}", self.program)),
            })
        }

        pub fn hang_up(self) {}
    }

    pub fn can_suspend() -> bool {
        false
    }

    pub fn suspend_self() -> Result<(), String> {
        Err("tuitor cannot stop here".into())
    }
}
