//! Directories as typed: `~` is the home directory, and a relative path starts at the
//! current project, so `/tab open ../web` opens the project next to it.

use std::path::{Path, PathBuf};

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
}
