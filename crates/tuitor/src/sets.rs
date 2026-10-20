//! Breakpoint sets, as commands: make one, switch the active one, put breakpoints in sets
//! and take them out, delete one. A set has no name but its colour, chosen when it is made
//! ([`palette`]); commands take the word for the colour, as `teal` or `pale-sky`, or the
//! colour itself, `#4cc2bd`. A breakpoint can be in several sets: two ways into one service
//! share its own.

use std::path::PathBuf;

use tuitor_core::Engine;
use tuitor_core::breakpoints::{BreakpointId, Breakpoints, SetId};
use tuitor_core::palette;

use crate::app::{App, Outcome, Refusal, canonical};
use crate::command::Location;
use crate::overlay::Overlay;
use crate::tab::TabId;

/// Names a message lists before it counts the rest.
const LISTED: usize = 4;

/// What `set` is called among the sets of `bps`: the word for its colour, numbered when an
/// earlier set's colour has the same word.
pub(crate) fn name(bps: &Breakpoints, set: SetId) -> String {
    palette::name(set, bps.sets())
}

/// The set of `bps` called `text`, or of the colour `#rrggbb`.
pub fn parse_set(bps: &Breakpoints, text: &str) -> Result<SetId, String> {
    palette::parse(text, bps.sets()).ok_or_else(|| {
        let text = text.trim();
        format!("no set `{text}`: sets go by colour, {}", names(bps, bps.sets()))
    })
}

/// "teal stops · rose, orchid record only": what each set does now.
pub(crate) fn roles(engine: &Engine) -> String {
    let bps = engine.breakpoints();
    let active = bps.active_set();
    let others: Vec<SetId> = bps.sets().iter().copied().filter(|s| *s != active).collect();
    let stops = format!("{} stops", name(bps, active));
    match others.as_slice() {
        [] => stops,
        [one] => format!("{stops} · {} records only", name(bps, *one)),
        few if few.len() <= LISTED => format!("{stops} · {} record only", names(bps, few)),
        many => format!("{stops} · the {} other sets record only", many.len()),
    }
}

/// The names of `sets`: "rose", "rose and teal", "rose, teal and sky"; past [`LISTED`],
/// "rose, teal, sky, gold and 3 more".
pub(crate) fn names(bps: &Breakpoints, sets: &[SetId]) -> String {
    let mut names: Vec<String> = sets.iter().take(LISTED).map(|s| name(bps, *s)).collect();
    if sets.len() > LISTED {
        names.push(format!("{} more", sets.len() - LISTED));
    }
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// The sets of a tab in a list, to make one active: `/set pick`.
pub struct SetPick {
    pub tab: TabId,
    pub selected: usize,
}

impl SetPick {
    pub fn select(&mut self, delta: isize, len: usize) {
        if len > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        }
    }
}

/// `[set] [file:line…]`: a set of `bps` by its colour, and places.
fn set_and_places(bps: &Breakpoints, arg: Option<&str>) -> Result<(Option<SetId>, Vec<Location>), String> {
    let mut set = None;
    let mut places = Vec::new();
    for word in arg.unwrap_or_default().split_whitespace() {
        if word.contains(':') {
            places.push(Location::parse(word)?);
        } else if set.is_none() {
            set = Some(parse_set(bps, word)?);
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
        let (named, places) = set_and_places(self.tab().engine.breakpoints(), places.as_deref())?;
        if named.is_some() {
            return Err("a new set gets a colour of its own; /set new takes only file:line places".into());
        }
        let here = if places.is_empty() { self.breakpoint_here().ok() } else { None };
        let set = self.tab_mut().engine.new_set();
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
        let engine = &self.tab().engine;
        let name = name(engine.breakpoints(), set);
        Ok(format!("new set {name}{with}, active; breakpoints added go to it · {}", roles(engine)))
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
        let name = name(engine.breakpoints(), set);
        engine.activate_set(set).map_err(|e| format!("{name}: {e}"))?;
        Ok(roles(engine))
    }

    /// `/set active ‹set›`: the set of that colour active; without one, `/set pick`.
    pub(crate) fn set_active_named(&mut self, arg: Option<String>) -> Outcome {
        match arg.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(text) => self.set_active(parse_set(self.tab().engine.breakpoints(), text)?),
            None => self.set_pick(),
        }
    }

    /// `/set pick`: the sets in a list, the active one selected, to make another active.
    pub(crate) fn set_pick(&mut self) -> Outcome {
        let bps = self.tab().engine.breakpoints();
        let selected = bps.sets().iter().position(|s| *s == bps.active_set()).unwrap_or(0);
        self.ui.overlay = Some(Overlay::Sets(SetPick { tab: self.tabs.active_id(), selected }));
        Ok(String::new())
    }

    /// `enter` in the list of sets: the one selected active.
    pub(crate) fn set_picked(&mut self) -> Outcome {
        let Some(Overlay::Sets(pick)) = self.ui.overlay.take() else { return Ok(String::new()) };
        let engine = self.engine_of(pick.tab)?;
        let set = *engine.breakpoints().sets().get(pick.selected).ok_or("that set is gone")?;
        let name = name(engine.breakpoints(), set);
        engine.activate_set(set).map_err(|e| format!("{name}: {e}"))?;
        Ok(roles(engine))
    }

    /// `/set add|remove|toggle [set] [file:line…]`: the breakpoints at the places, else the one
    /// selected or on the SOURCE line, join the set (the active one) or leave it. `add` makes a
    /// breakpoint in that set alone at a place that has none.
    pub(crate) fn set_membership(&mut self, how: Membership, arg: Option<String>) -> Outcome {
        let (named, places) = set_and_places(self.tab().engine.breakpoints(), arg.as_deref())?;
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
        let bps = self.tab().engine.breakpoints();
        let records = how != Membership::Remove && set != bps.active_set();
        let note = if records { format!(" · {} records only", name(bps, set)) } else { String::new() };
        Ok(format!("{}{note}", said.join(" · ")))
    }

    /// A breakpoint joins `set` or leaves it, or one is made at a place in it alone; says
    /// which sets it is in then.
    fn change_membership(&mut self, how: Membership, target: Target, set: SetId) -> Result<String, Refusal> {
        let set_name = name(self.tab().engine.breakpoints(), set);
        let failed = |e: anyhow::Error| format!("{set_name}: {e}");
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
        Ok(format!("#{id} is in {}", names(engine.breakpoints(), &sets)))
    }

    /// `/set delete [set] [to]`: the set named, else the one the selected breakpoint is shown
    /// in, else the active one. Its breakpoints join `to`; without, those in no other set go
    /// with it, once asked for twice in a row.
    pub(crate) fn set_delete(&mut self, arg: Option<String>) -> Outcome {
        let mut words = arg.as_deref().unwrap_or_default().split_whitespace();
        let bps = self.tab().engine.breakpoints();
        let named = words.next().map(|word| parse_set(bps, word)).transpose()?;
        let to = words.next().map(|word| parse_set(bps, word)).transpose()?;
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
            let name = name(self.engine_of(tab)?.breakpoints(), set);
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
        let (gone, to_name) = (name(engine.breakpoints(), set), to.map(|to| name(engine.breakpoints(), to)));
        let deleted = engine.delete_set(set, to).map_err(|e| format!("{gone}: {e}"))?;
        let what = match (to_name, deleted) {
            (Some(to), _) => format!("; its breakpoints are in {to}"),
            (None, 0) => String::new(),
            (None, 1) => " with its breakpoint".to_owned(),
            (None, n) => format!(" with its {n} breakpoints"),
        };
        Ok(format!("deleted {gone}{what} · {}", roles(engine)))
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

    /// Rose, teal, orchid and sky, as sets were coloured before their colours were chosen.
    fn old_sets() -> (Breakpoints, Vec<SetId>) {
        let sets: Vec<SetId> = palette::OLD[..4].iter().map(|(rgb, _)| SetId::from_rgb(*rgb)).collect();
        (Breakpoints::from_saved(Vec::new(), &sets, None), sets)
    }

    #[test]
    fn a_set_and_places_are_told_apart() {
        let (bps, sets) = old_sets();
        let (set, places) = set_and_places(&bps, Some("teal orders.php:68 :12")).expect("parse");
        assert_eq!((set, places.len()), (Some(sets[1]), 2));
        assert_eq!(places[1], Location { file: None, line: 12 });
        assert_eq!(set_and_places(&bps, None), Ok((None, vec![])));
        assert!(set_and_places(&bps, Some("teal rose")).is_err());
        assert_eq!(names(&bps, &[sets[0], sets[1], sets[3]]), "rose, teal and sky");
    }

    #[test]
    fn sets_go_by_their_colour() {
        let (bps, sets) = old_sets();
        assert_eq!(parse_set(&bps, "rose"), Ok(sets[0]));
        assert_eq!(parse_set(&bps, " Teal "), Ok(sets[1]));
        assert_eq!(parse_set(&bps, "#7ea9ee"), Ok(sets[3]));
        assert!(parse_set(&bps, "red").is_err_and(|e| e.contains("rose, teal, orchid and sky")));
        assert!(parse_set(&bps, "gold").is_err(), "a colour no set has");
    }

    #[test]
    fn many_sets_are_counted_rather_than_named() {
        let mut bps = Breakpoints::default();
        for _ in 0..11 {
            bps.new_set();
        }
        let named = names(&bps, bps.sets());
        assert!(named.ends_with(" and 8 more") && named.matches(", ").count() == 3, "{named}");
        // Every set has a name of its own, which finds it again.
        for set in bps.sets() {
            assert_eq!(parse_set(&bps, &name(&bps, *set)), Ok(*set));
        }
    }
}
