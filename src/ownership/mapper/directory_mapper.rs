use std::sync::Arc;

use super::escaper::escape_brackets;
use super::{Entry, Source};
use super::{Mapper, OwnerMatcher};
use crate::project::Project;

pub struct DirectoryMapper {
    project: Arc<Project>,
}

impl DirectoryMapper {
    pub fn build(project: Arc<Project>) -> Self {
        Self { project }
    }
}

// Build the glob for a `.codeowner` directory. A repo-root `.codeowner` has an
// empty directory root, so it becomes a catch-all `**` that owns every file
// no more specific source claims. This matches the per-request resolver, which
// walks up to the repo root and honors the root `.codeowner`.
fn directory_glob(dir_root: &str) -> String {
    let escaped = escape_brackets(dir_root);
    if escaped.is_empty() {
        "**".to_owned()
    } else {
        format!("{escaped}/**/**")
    }
}

impl Mapper for DirectoryMapper {
    fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = Vec::new();
        let team_by_name = self.project.teams_by_name.clone();

        for directory_codeowner_file in &self.project.directory_codeowner_files {
            let dir_root = directory_codeowner_file
                .directory_root()
                .map(|p| p.to_string_lossy())
                .unwrap_or_default();
            let team = team_by_name.get(&directory_codeowner_file.owner);
            if let Some(team) = team {
                entries.push(Entry {
                    path: directory_glob(&dir_root),
                    github_team: team.github_team.to_owned(),
                    team_name: team.name.to_owned(),
                    disabled: team.avoid_ownership,
                });
            }
        }

        entries
    }

    fn owner_matchers(&self) -> Vec<OwnerMatcher> {
        let mut owner_matchers = Vec::new();

        for file in &self.project.directory_codeowner_files {
            let dir_root = file.directory_root().map(|p| p.to_string_lossy()).unwrap_or_default();
            owner_matchers.push(OwnerMatcher::new_glob(
                directory_glob(&dir_root),
                file.owner.to_owned(),
                Source::Directory(dir_root.to_string()),
            ));
        }

        owner_matchers
    }

    fn name(&self) -> String {
        "Owner in .codeowner".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::common_test::tests::{
        build_ownership_with_directory_codeowners, build_ownership_with_directory_codeowners_with_brackets,
        build_ownership_with_root_directory_codeowner, vecs_match,
    };
    use crate::ownership::file_owner_finder::FileOwnerFinder;
    use std::path::Path;

    use super::*;
    #[test]
    fn test_entries() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_directory_codeowners()?;
        let mapper = DirectoryMapper::build(ownership.project.clone());
        vecs_match(
            &mapper.entries(),
            &vec![
                Entry {
                    path: "app/consumers/**/**".to_owned(),
                    github_team: "@Bar".to_owned(),
                    team_name: "Bar".to_owned(),
                    disabled: false,
                },
                Entry {
                    path: "app/services/**/**".to_owned(),
                    github_team: "@Foo".to_owned(),
                    team_name: "Foo".to_owned(),
                    disabled: false,
                },
                Entry {
                    path: "app/services/exciting/**/**".to_owned(),
                    github_team: "@Bar".to_owned(),
                    team_name: "Bar".to_owned(),
                    disabled: false,
                },
            ],
        );
        Ok(())
    }

    #[test]
    fn test_entries_with_brackets() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_directory_codeowners_with_brackets()?;
        let mapper = DirectoryMapper::build(ownership.project.clone());
        vecs_match(
            &mapper.entries(),
            &vec![
                Entry {
                    path: "app/\\[consumers\\]/**/**".to_string(),
                    github_team: "@Bar".to_string(),
                    team_name: "Bar".to_string(),
                    disabled: false,
                },
                Entry {
                    path: "app/\\[consumers\\]/deep/nesting/\\[nestdir\\]/**/**".to_string(),
                    github_team: "@Foo".to_string(),
                    team_name: "Foo".to_string(),
                    disabled: false,
                },
            ],
        );
        Ok(())
    }

    #[test]
    fn test_root_codeowner_entry_is_catch_all() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_root_directory_codeowner()?;
        let mapper = DirectoryMapper::build(ownership.project.clone());
        vecs_match(
            &mapper.entries(),
            &vec![
                Entry {
                    path: "**".to_owned(),
                    github_team: "@Bar".to_owned(),
                    team_name: "Bar".to_owned(),
                    disabled: false,
                },
                Entry {
                    path: "app/services/**/**".to_owned(),
                    github_team: "@Foo".to_owned(),
                    team_name: "Foo".to_owned(),
                    disabled: false,
                },
            ],
        );
        Ok(())
    }

    #[test]
    fn test_root_codeowner_owns_unclaimed_files_but_specific_wins() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_root_directory_codeowner()?;
        let mapper = DirectoryMapper::build(ownership.project.clone());
        let owner_matchers = mapper.owner_matchers();
        let finder = FileOwnerFinder {
            owner_matchers: &owner_matchers,
        };

        // A file no specific `.codeowner` claims falls back to the root owner.
        let unclaimed = finder.find(Path::new("lib/unclaimed/deep_file.rb"));
        assert_eq!(unclaimed.len(), 1);
        assert_eq!(unclaimed[0].team_name, "Bar");

        // A more specific `.codeowner` still wins over the root catch-all.
        let specific = finder.find(Path::new("app/services/service_file.rb"));
        assert_eq!(specific.len(), 1);
        assert_eq!(specific[0].team_name, "Foo");

        Ok(())
    }

    #[test]
    fn test_owner_matchers() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_directory_codeowners()?;
        let mapper = DirectoryMapper::build(ownership.project.clone());
        vecs_match(
            &mapper.owner_matchers(),
            &vec![
                OwnerMatcher::new_glob(
                    "app/consumers/**/**".to_owned(),
                    "Bar".to_owned(),
                    Source::Directory("app/consumers".to_string()),
                ),
                OwnerMatcher::new_glob(
                    "app/services/**/**".to_owned(),
                    "Foo".to_owned(),
                    Source::Directory("app/services".to_owned()),
                ),
                OwnerMatcher::new_glob(
                    "app/services/exciting/**/**".to_owned(),
                    "Bar".to_owned(),
                    Source::Directory("app/services/exciting".to_owned()),
                ),
            ],
        );
        Ok(())
    }

    #[test]
    fn test_owner_matchers_with_brackets() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_directory_codeowners_with_brackets()?;
        let mapper = DirectoryMapper::build(ownership.project.clone());
        vecs_match(
            &mapper.owner_matchers(),
            &vec![
                OwnerMatcher::new_glob(
                    "app/\\[consumers\\]/**/**".to_string(),
                    "Bar".to_string(),
                    Source::Directory("app/[consumers]".to_string()),
                ),
                OwnerMatcher::new_glob(
                    "app/\\[consumers\\]/deep/nesting/\\[nestdir\\]/**/**".to_string(),
                    "Foo".to_string(),
                    Source::Directory("app/[consumers]/deep/nesting/[nestdir]".to_string()),
                ),
            ],
        );
        Ok(())
    }
}
