//! Breakpoint sets, as commands: make one, switch the active one, move breakpoints between
//! them, delete one. A set has no name but its colour, which is what these take.

use tuitor_core::Engine;
use tuitor_core::breakpoints::{BreakpointId, SetId};

use crate::app::{App, Outcome, Refusal};
use crate::tab::TabId;
use crate::ui::theme::{SETS, set_name};

/// The set of the colour `name`.
pub fn parse_set(name: &str) -> Result<SetId, String> {
    let name = name.trim();
    let index = SETS.iter().position(|(colour, _)| *colour == name);
    index.and_then(|i| SetId::try_from(i).ok()).ok_or_else(|| {
        let colours: Vec<&str> = SETS.iter().map(|(colour, _)| *colour).collect();
        format!("no set `{name}`: sets go by colour, {}", colours.join(", "))
    })
}

/// "teal stops · rose, orchid record only": what each set does now.
fn roles(engine: &Engine) -> String {
    let bps = engine.breakpoints();
    let active = bps.active_set();
    let others: Vec<&str> = bps.sets().iter().filter(|s| **s != active).map(|s| set_name(*s)).collect();
    match others.as_slice() {
        [] => format!("{} stops", set_name(active)),
        [one] => format!("{} stops · {one} records only", set_name(active)),
        many => format!("{} stops · {} record only", set_name(active), many.join(", ")),
    }
}

impl App {
    pub(crate) fn set_new(&mut self) -> Outcome {
        let engine = &mut self.tab_mut().engine;
        let set = engine.new_set().map_err(|e| e.to_string())?;
        Ok(format!("new set {}, active; breakpoints added go to it · {}", set_name(set), roles(engine)))
    }

    /// Makes the set `delta` places on active.
    pub(crate) fn set_cycle(&mut self, delta: isize) -> Outcome {
        let bps = self.tab().engine.breakpoints();
        if bps.sets().len() == 1 {
            return Err(self.one_set());
        }
        let set = bps.neighbour(bps.active_set(), delta);
        self.set_active(set)
    }

    pub(crate) fn set_active(&mut self, set: SetId) -> Outcome {
        let engine = &mut self.tab_mut().engine;
        engine.activate_set(set).map_err(|e| format!("{}: {e}", set_name(set)))?;
        Ok(roles(engine))
    }

    /// Moves the breakpoint selected in BREAKPOINTS, or on the SOURCE line, to `to`, or to the
    /// set after its own.
    pub(crate) fn set_move(&mut self, to: Option<String>) -> Outcome {
        let to = to.as_deref().map(parse_set).transpose()?;
        let (tab, id) = self.breakpoint_here()?;
        let engine = self.engine_of(tab)?;
        let bps = engine.breakpoints();
        let from = bps.get(id).map(|b| b.set).ok_or("that breakpoint is gone")?;
        if to.is_none() && bps.sets().len() == 1 {
            return Err(self.one_set());
        }
        let to = to.unwrap_or_else(|| bps.neighbour(from, 1));
        engine.move_breakpoint(id, to).map_err(|e| format!("{}: {e}", set_name(to)))?;
        let note = if to == engine.breakpoints().active_set() { "it stops" } else { "it records only" };
        Ok(format!("#{id} is in {} now: {note}", set_name(to)))
    }

    /// `/set delete [set] [to]`: the set named, else that of the selected breakpoint, else the
    /// active one. Its breakpoints go to `to`, or, asked for twice in a row, with it.
    pub(crate) fn set_delete(&mut self, arg: Option<String>) -> Outcome {
        let mut words = arg.as_deref().unwrap_or_default().split_whitespace();
        let named = words.next().map(parse_set).transpose()?;
        let to = words.next().map(parse_set).transpose()?;
        let (tab, set) = match named {
            Some(set) => (self.tabs.active_id(), set),
            None => match self.selected_bp() {
                Some((tab, id)) => {
                    let set = self.engine_of(tab)?.breakpoints().get(id).map(|b| b.set);
                    (tab, set.ok_or("that breakpoint is gone")?)
                }
                None => (self.tabs.active_id(), self.tab().engine.breakpoints().active_set()),
            },
        };
        let count = self.engine_of(tab)?.breakpoints().in_set(set).count();
        if to.is_none() && count > 0 && self.ui.deleting != Some((tab, set)) {
            self.ui.deleting = Some((tab, set));
            let again = self.key_or_command("set delete");
            let name = set_name(set);
            let (them, pronoun) = if count == 1 {
                ("its breakpoint".to_owned(), "it")
            } else {
                (format!("its {count} breakpoints"), "them")
            };
            return Err(Refusal::warn(format!(
                "{again} again deletes {name} with {them}; /set delete {name} ‹set› moves {pronoun} there"
            )));
        }
        self.ui.deleting = None;
        let engine = self.engine_of(tab)?;
        let deleted = engine.delete_set(set, to).map_err(|e| format!("{}: {e}", set_name(set)))?;
        let what = match (to, deleted) {
            (Some(to), _) if count > 0 => format!("; its breakpoints are in {}", set_name(to)),
            (_, 0) => String::new(),
            (_, 1) => " with its breakpoint".to_owned(),
            (_, n) => format!(" with its {n} breakpoints"),
        };
        Ok(format!("deleted {}{what} · {}", set_name(set), roles(engine)))
    }

    /// The breakpoint selected in BREAKPOINTS, else the one on the SOURCE line.
    fn breakpoint_here(&self) -> Result<(TabId, BreakpointId), Refusal> {
        if let Some(selected) = self.selected_bp() {
            return Ok(selected);
        }
        let path = self.breakpoint_file()?;
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let tab = self.tab();
        let line = tab.ui.source.cursor.max(1);
        let bp = tab.engine.breakpoints().at(&path, line).ok_or("no breakpoint on this line; b adds one")?;
        Ok((self.tabs.active_id(), bp.id))
    }

    fn one_set(&self) -> Refusal {
        Refusal::warn(format!("there is one set so far; {} makes another", self.key_or_command("set new")))
    }

    /// The key that runs `command` here, else the command line.
    fn key_or_command(&self, command: &str) -> String {
        self.keymap.key_for(self.focus().name(), command).unwrap_or_else(|| format!("/{command}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_go_by_their_colour() {
        assert_eq!(parse_set("rose"), Ok(0));
        assert_eq!(parse_set(" teal "), Ok(1));
        assert!(parse_set("red").is_err_and(|e| e.contains("rose, teal")));
    }
}
