//! Breakpoint sets, as commands: make one, switch the active one, put breakpoints in sets
//! and take them out, delete one. A set has no name but its colour, which is what these
//! take. A breakpoint can be in several sets: two ways into one service share its own.

use std::path::PathBuf;

use tuitor_core::Engine;
use tuitor_core::breakpoints::{BreakpointId, SetId};

use crate::app::{App, Outcome, Refusal, canonical};
use crate::command::Location;
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

/// The names of `sets`: "rose", "rose and teal", "rose, teal and sky".
fn names(sets: &[SetId]) -> String {
    let names: Vec<&str> = sets.iter().map(|s| set_name(*s)).collect();
    match names.split_last() {
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// `[set] [file:line…]`: a set by its colour, and places.
fn set_and_places(arg: Option<&str>) -> Result<(Option<SetId>, Vec<Location>), String> {
    let mut set = None;
    let mut places = Vec::new();
    for word in arg.unwrap_or_default().split_whitespace() {
        if word.contains(':') {
            places.push(Location::parse(word)?);
        } else if set.is_none() {
            set = Some(parse_set(word)?);
        } else {
            return Err(format!("one set at a time, then file:line places; `{word}` is neither"));
        }
    }
    Ok((set, places))
}

/// A breakpoint to put in a set or take out, or a place to make one in it.
enum Target {
    Breakpoint(TabId, BreakpointId),
    Place(PathBuf, u32),
}

/// What `set add`, `set remove` and `set toggle` do with a breakpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    Add,
    Remove,
    Toggle,
}

impl App {
    /// `/set new [file:line…]`: a set in a new colour, made active. The breakpoints at the
    /// places, made where there are none, else the breakpoint selected or on the SOURCE line,
    /// if there is one, are in it from the start.
    pub(crate) fn set_new(&mut self, places: Option<String>) -> Outcome {
        let (named, places) = set_and_places(places.as_deref())?;
        if named.is_some() {
            return Err("a new set takes the first free colour; /set new takes only file:line places".into());
        }
        let here = if places.is_empty() { self.breakpoint_here().ok() } else { None };
        let set = self.tab_mut().engine.new_set().map_err(|e| e.to_string())?;
        let mut joined = Vec::new();
        for place in &places {
            let (path, line) = self.place(place)?;
            joined.push(self.tab_mut().engine.put_in_set(&path, line, set).map_err(|e| e.to_string())?.0);
        }
        if let Some((tab, id)) = here {
            self.engine_of(tab)?.join_set(id, set).map_err(|e| e.to_string())?;
            joined.push(id);
        }
        let with = match joined.as_slice() {
            [] => String::new(),
            ids => format!(" with {}", ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ")),
        };
        let roles = roles(&self.tab().engine);
        Ok(format!("new set {}{with}, active; breakpoints added go to it · {roles}", set_name(set)))
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

    /// `/set add|remove|toggle [set] [file:line…]`: the breakpoints at the places, else the one
    /// selected or on the SOURCE line, join the set (the active one) or leave it. `add` makes a
    /// breakpoint in that set alone at a place that has none.
    pub(crate) fn set_membership(&mut self, how: Membership, arg: Option<String>) -> Outcome {
        let (named, places) = set_and_places(arg.as_deref())?;
        let set = named.unwrap_or_else(|| self.tab().engine.breakpoints().active_set());
        let mut targets = Vec::new();
        if places.is_empty() {
            let (tab, id) = self.breakpoint_here()?;
            targets.push(Target::Breakpoint(tab, id));
        }
        for place in &places {
            let (path, line) = self.place(place)?;
            targets.push(match self.tab().engine.breakpoints().at(&canonical(&path), line) {
                Some(bp) => Target::Breakpoint(self.tabs.active_id(), bp.id),
                None => Target::Place(path, line),
            });
        }
        let mut said = Vec::new();
        for target in targets {
            said.push(self.change_membership(how, target, set)?);
        }
        // Joining a set that is not active makes no breakpoint stop.
        let records = how != Membership::Remove && set != self.tab().engine.breakpoints().active_set();
        let note = if records { format!(" · {} records only", set_name(set)) } else { String::new() };
        Ok(format!("{}{note}", said.join(" · ")))
    }

    /// A breakpoint joins `set` or leaves it, or one is made at a place in it alone; says
    /// which sets it is in then.
    fn change_membership(&mut self, how: Membership, target: Target, set: SetId) -> Result<String, Refusal> {
        let failed = |e: anyhow::Error| format!("{}: {e}", set_name(set));
        let (engine, id) = match target {
            Target::Breakpoint(tab, id) => {
                let engine = self.engine_of(tab)?;
                let in_set = engine.breakpoints().get(id).is_some_and(|b| b.in_set(set));
                if how == Membership::Remove || (how == Membership::Toggle && in_set) {
                    engine.leave_set(id, set).map_err(failed)?;
                } else {
                    engine.join_set(id, set).map_err(failed)?;
                }
                (engine, id)
            }
            Target::Place(_, _) if how == Membership::Remove => return Err("no breakpoint there".into()),
            Target::Place(path, line) => {
                let engine = &mut self.tab_mut().engine;
                let (id, _) = engine.put_in_set(&path, line, set).map_err(failed)?;
                (engine, id)
            }
        };
        let sets = engine.breakpoints().get(id).map(|b| b.sets.clone()).unwrap_or_default();
        Ok(format!("#{id} is in {}", names(&sets)))
    }

    /// `/set delete [set] [to]`: the set named, else the one the selected breakpoint is shown
    /// in, else the active one. Its breakpoints join `to`; without, those in no other set go
    /// with it, once asked for twice in a row.
    pub(crate) fn set_delete(&mut self, arg: Option<String>) -> Outcome {
        let mut words = arg.as_deref().unwrap_or_default().split_whitespace();
        let named = words.next().map(parse_set).transpose()?;
        let to = words.next().map(parse_set).transpose()?;
        let (tab, set) = match named {
            Some(set) => (self.tabs.active_id(), set),
            None => match self.selected_bp() {
                Some((tab, id)) => {
                    let bps = self.engine_of(tab)?.breakpoints();
                    let set = bps.get(id).map(|b| b.shown_set(bps.active_set()));
                    (tab, set.ok_or("that breakpoint is gone")?)
                }
                None => (self.tabs.active_id(), self.tab().engine.breakpoints().active_set()),
            },
        };
        let alone = self.engine_of(tab)?.breakpoints().members(set).filter(|b| b.sets.len() == 1).count();
        if to.is_none() && alone > 0 && self.ui.deleting != Some((tab, set)) {
            self.ui.deleting = Some((tab, set));
            let again = self.key_or_command("set delete");
            let name = set_name(set);
            let (them, pronoun) = if alone == 1 {
                ("its breakpoint in no other set".to_owned(), "it")
            } else {
                (format!("its {alone} breakpoints in no other set"), "them")
            };
            return Err(Refusal::warn(format!(
                "{again} again deletes {name} with {them}; /set delete {name} ‹set› moves {pronoun} there"
            )));
        }
        self.ui.deleting = None;
        let engine = self.engine_of(tab)?;
        let deleted = engine.delete_set(set, to).map_err(|e| format!("{}: {e}", set_name(set)))?;
        let what = match (to, deleted) {
            (Some(to), _) => format!("; its breakpoints are in {}", set_name(to)),
            (None, 0) => String::new(),
            (None, 1) => " with its breakpoint".to_owned(),
            (None, n) => format!(" with its {n} breakpoints"),
        };
        Ok(format!("deleted {}{what} · {}", set_name(set), roles(engine)))
    }

    /// A place as typed, `file:line` or `:line` in the file in SOURCE.
    fn place(&self, place: &Location) -> Result<(PathBuf, u32), Refusal> {
        let path = match &place.file {
            Some(file) => self.resolve_file(file)?,
            None => self.breakpoint_file()?,
        };
        Ok((path, place.line))
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
    fn a_set_and_places_are_told_apart() {
        let (set, places) = set_and_places(Some("teal orders.php:68 :12")).expect("parse");
        assert_eq!((set, places.len()), (Some(1), 2));
        assert_eq!(places[1], Location { file: None, line: 12 });
        assert_eq!(set_and_places(None), Ok((None, vec![])));
        assert!(set_and_places(Some("teal rose")).is_err());
        assert_eq!(names(&[0, 1, 3]), "rose, teal and sky");
    }

    #[test]
    fn sets_go_by_their_colour() {
        assert_eq!(parse_set("rose"), Ok(0));
        assert_eq!(parse_set(" teal "), Ok(1));
        assert!(parse_set("red").is_err_and(|e| e.contains("rose, teal")));
    }
}
