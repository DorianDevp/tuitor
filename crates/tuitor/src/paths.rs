//! Paths as typed. For a directory `~` is the home directory, and a relative path starts at
//! the current project, so `/tab open ../web` opens the project next to it. A file, as in
//! `/bp toggle LoyaltyListener.php:16`, may also be named by the end of its path.

use std::path::{Path, PathBuf};

use tuitor_core::config::home_relative;

/// Files listed when a command names more than one.
const LISTED: usize = 4;

/// The file `typed` names: absolute, or from the project's directory `dir`; else the one file
/// of the project whose path ends with it, whole names only, so that `LoyaltyListener.php`
/// and `Event/LoyaltyListener.php` find `src/Event/LoyaltyListener.php` and `Listener.php`
/// does not. `files` lists the project's files relative to `dir`, as the finder has them
/// (what .gitignore leaves out is not there), and is called only when it is needed. Several
/// files, or none, are an error that says which.
pub fn file(typed: &str, dir: &Path, files: impl FnOnce() -> Vec<String>) -> Result<PathBuf, String> {
    let given = Path::new(typed);
    let path = if given.is_absolute() { given.to_path_buf() } else { dir.join(given) };
    if path.is_file() {
        return Ok(path);
    }
    // An empty tab has no project to look in.
    if given.is_absolute() || dir.as_os_str().is_empty() {
        return Err(format!("no such file: {}", home_relative(&path)));
    }
    let files = files();
    match ending_with(&files, typed).as_slice() {
        [one] => Ok(dir.join(one)),
        [] => Err(format!("no file `{typed}` in {}", home_relative(dir))),
        many => {
            let more = many.len().saturating_sub(LISTED);
            let more = if more > 0 { format!(" and {more} more") } else { String::new() };
            let listed = many[..many.len().min(LISTED)].join(", ");
            Err(format!("{} files end with `{typed}`: {listed}{more}; give more of the path", many.len()))
        }
    }
}

/// The paths of `files` that end with `typed`, compared name by name.
fn ending_with<'a>(files: &'a [String], typed: &str) -> Vec<&'a str> {
    let typed = Path::new(typed.trim_start_matches("./"));
    if typed.file_name().is_none() {
        return Vec::new();
    }
    files.iter().map(String::as_str).filter(|file| Path::new(file).ends_with(typed)).collect()
}

pub fn expand(text: &str, home: Option<&Path>, base: &Path) -> PathBuf {
    let text = text.trim();
    let path = match (text.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(text),
    };
    if path.is_absolute() { path } else { base.join(path) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_is_home_and_relative_paths_start_at_the_project() {
        let home = Some(Path::new("/home/me"));
        let base = Path::new("/home/me/shop/api");
        assert_eq!(expand("~", home, base), Path::new("/home/me"));
        assert_eq!(expand(" ~/shop/web ", home, base), Path::new("/home/me/shop/web"));
        assert_eq!(expand("../web", home, base), Path::new("/home/me/shop/api/../web"));
        assert_eq!(expand("/srv/app", home, base), Path::new("/srv/app"));
        assert_eq!(expand("~other/x", home, base), Path::new("/home/me/shop/api/~other/x"));
        assert_eq!(expand("~/x", None, base), Path::new("/home/me/shop/api/~/x"));
    }

    /// A project in the temporary directory with `files`, and `ignored` that .gitignore leaves
    /// out; removed when dropped.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, files: &[&str], ignored: &[&str]) -> Project {
            let dir = std::env::temp_dir().join(format!("tuitor-paths-{name}-{}", std::process::id()));
            std::fs::create_dir_all(dir.join(".git")).expect("project");
            for file in files.iter().chain(ignored) {
                let path = dir.join(file);
                std::fs::create_dir_all(path.parent().expect("a directory")).expect("directories");
                std::fs::write(path, "<?php").expect("file");
            }
            std::fs::write(dir.join(".gitignore"), ignored.join("\n")).expect(".gitignore");
            Project(dir)
        }

        fn file(&self, typed: &str) -> Result<PathBuf, String> {
            file(typed, &self.0, || crate::overlay::workspace_files(&self.0))
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn a_file_is_found_by_the_end_of_its_path() {
        let project = Project::new(
            "end",
            &["src/Event/LoyaltyListener.php", "src/Service/OrderService.php", "public/index.php"],
            &[],
        );
        let listener = project.0.join("src/Event/LoyaltyListener.php");
        assert_eq!(project.file("src/Event/LoyaltyListener.php"), Ok(listener.clone()));
        assert_eq!(project.file("LoyaltyListener.php"), Ok(listener.clone()));
        assert_eq!(project.file("Event/LoyaltyListener.php"), Ok(listener.clone()));
        assert_eq!(project.file("./LoyaltyListener.php"), Ok(listener.clone()));
        assert_eq!(project.file(&listener.display().to_string()), Ok(listener));
    }

    #[test]
    fn several_files_or_none_say_so() {
        let project = Project::new(
            "several",
            &["src/Event/OrderListener.php", "tests/Event/OrderListener.php", "src/Entity/Order.php"],
            &["vendor/acme/Kernel.php"],
        );
        let several = project.file("OrderListener.php").expect_err("two files");
        assert!(several.contains("2 files end with `OrderListener.php`"), "{several}");
        assert!(several.contains("src/Event/OrderListener.php") && several.contains("tests/Event/OrderListener.php"));
        // More of the path picks one.
        assert_eq!(project.file("tests/Event/OrderListener.php"), Ok(project.0.join("tests/Event/OrderListener.php")));
        // Whole names only: `Order.php` is not the end of `OrderListener.php`.
        assert_eq!(project.file("Order.php"), Ok(project.0.join("src/Entity/Order.php")));
        let none = project.file("Listener.php").expect_err("no such file");
        assert!(none.starts_with("no file `Listener.php` in "), "{none}");
        // What .gitignore leaves out is found only by its path.
        assert!(project.file("Kernel.php").is_err());
        assert_eq!(project.file("vendor/acme/Kernel.php"), Ok(project.0.join("vendor/acme/Kernel.php")));
        assert!(project.file("/nowhere/Order.php").expect_err("absolute").starts_with("no such file: /nowhere"));
    }

    #[test]
    fn many_files_are_listed_in_part() {
        let files: Vec<String> = (1..=6).map(|n| format!("m{n}/index.php")).collect();
        let error = file("index.php", Path::new("/nonexistent/shop"), || files).expect_err("six files");
        assert_eq!(
            error,
            "6 files end with `index.php`: m1/index.php, m2/index.php, m3/index.php, m4/index.php and 2 more; \
             give more of the path"
        );
        // Without a project there is nothing to look through.
        let error = file("index.php", Path::new(""), || unreachable!("no listing")).expect_err("empty tab");
        assert_eq!(error, "no such file: index.php");
    }
}
