use file_owner_finder::FileOwnerFinder;
use itertools::Itertools;
use mapper::{OwnerMatcher, Source, TeamName};
use serde::Serialize;
use std::{
    error::Error,
    fmt::{self, Display},
    path::Path,
    sync::Arc,
};
use tracing::{info, instrument};

pub(crate) mod codeowners_file_parser;
pub(crate) mod codeowners_query;
mod file_generator;
mod file_owner_finder;
pub mod file_owner_resolver;
pub(crate) mod mapper;
mod validator;

use crate::{
    ownership::mapper::DirectoryMapper,
    project::{Project, Team},
};

pub use validator::Errors as ValidatorErrors;

use self::{
    codeowners_file_parser::parse_for_team,
    file_generator::FileGenerator,
    mapper::{JavascriptPackageMapper, Mapper, RubyPackageMapper, TeamFileMapper, TeamGemMapper, TeamGlobMapper, TeamYmlMapper},
    validator::Validator,
};

pub struct Ownership {
    project: Arc<Project>,
}
#[derive(Debug, Clone)]
pub struct FileOwner {
    pub team: Team,
    pub team_config_file_path: String,
    pub sources: Vec<Source>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TeamOwnership {
    pub heading: String,
    pub globs: Vec<String>,
}

impl TeamOwnership {
    fn new(heading: String) -> Self {
        Self {
            heading,
            ..Default::default()
        }
    }
}

impl Display for FileOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sources = if self.sources.is_empty() {
            "".to_string()
        } else {
            let sources_str = self
                .sources
                .iter()
                .sorted_by_key(|source| source.to_string())
                .map(|source| source.to_string())
                .collect::<Vec<_>>()
                .join("\n- ");
            format!("\n- {}", sources_str)
        };

        write!(
            f,
            "Team: {}\nGithub Team: {}\nTeam YML: {}\nDescription:{}",
            self.team.name, self.team.github_team, self.team_config_file_path, sources
        )
    }
}

impl Default for FileOwner {
    fn default() -> Self {
        Self {
            team: Team {
                name: "Unowned".to_string(),
                github_team: "Unowned".to_string(),
                ..Default::default()
            },
            team_config_file_path: "".to_string(),
            sources: vec![],
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Clone, Serialize)]
pub struct Entry {
    pub path: String,
    pub github_team: String,
    pub team_name: TeamName,
    pub disabled: bool,
}

impl Entry {
    fn to_row(&self) -> String {
        let line = format!("/{} {}", self.path, self.github_team);
        if self.disabled { format!("# {}", line) } else { line }
    }
}

/// The ownership entries a single mapper produces, grouped under the mapper's
/// display name. This is the structured form of one section of the generated
/// CODEOWNERS file. Clients that need a per-mapper map of glob to owner (for
/// example a custom CODEOWNERS generator) read this instead of parsing the file.
#[derive(Debug, Clone, Serialize)]
pub struct MapperOwnership {
    pub mapper: String,
    pub entries: Vec<Entry>,
}

impl Ownership {
    pub fn build(project: Project) -> Self {
        Self {
            project: Arc::new(project),
        }
    }

    #[instrument(name = "ownership_validate", level = "debug", skip_all)]
    pub fn validate(&self) -> Result<(), ValidatorErrors> {
        info!("validating file ownership");
        let validator = Validator {
            project: self.project.clone(),
            mappers: self.mappers(),
            file_generator: FileGenerator { mappers: self.mappers() },
            executable_name: self.project.executable_name.clone(),
            allow_ownership_override: self.project.allow_ownership_override,
        };

        validator.validate()
    }

    #[instrument(level = "debug", skip_all)]
    pub fn for_file(&self, file_path: &Path) -> Result<Vec<FileOwner>, ValidatorErrors> {
        info!("getting file ownership for {}", file_path.display());
        let owner_matchers: Vec<OwnerMatcher> = self.mappers().iter().flat_map(|mapper| mapper.owner_matchers()).collect();
        let file_owner_finder = FileOwnerFinder {
            owner_matchers: &owner_matchers,
        };
        let owners = file_owner_finder.find(Path::new(file_path));
        Ok(owners
            .iter()
            .sorted_by_key(|owner| owner.team_name.to_lowercase())
            .map(|owner| match self.project.get_team(&owner.team_name) {
                Some(team) => FileOwner {
                    team: team.clone(),
                    team_config_file_path: team
                        .path
                        .strip_prefix(&self.project.base_path)
                        .map_or_else(|_| String::new(), |p| p.to_string_lossy().to_string()),
                    sources: owner.sources.clone(),
                },
                None => FileOwner::default(),
            })
            .collect())
    }

    #[instrument(level = "debug", skip_all)]
    pub fn for_team(&self, team_name: &str) -> Result<Vec<TeamOwnership>, Box<dyn Error>> {
        info!("getting team ownership for {}", team_name);
        let team = self.project.get_team(team_name).ok_or("Team not found")?;
        let codeowners_file = self.project.get_codeowners_file()?;

        parse_for_team(team.github_team, &codeowners_file)
    }

    #[instrument(level = "debug", skip_all)]
    pub fn generate_file(&self) -> String {
        info!("generating codeowners file");
        let file_generator = FileGenerator { mappers: self.mappers() };
        file_generator.generate_file()
    }

    /// Returns the ownership entries each mapper produces, in mapper order. This
    /// is the same data the CODEOWNERS generator writes, but structured per
    /// mapper instead of joined into one file, so a caller can build its own
    /// output (for example a GitLab-flavored CODEOWNERS file).
    #[instrument(level = "debug", skip_all)]
    pub fn ownership_entries(&self) -> Vec<MapperOwnership> {
        self.mappers()
            .iter()
            .map(|mapper| MapperOwnership {
                mapper: mapper.name(),
                entries: mapper.entries(),
            })
            .collect()
    }

    #[instrument(name = "mapper_build", level = "debug", skip_all)]
    fn mappers(&self) -> Vec<Box<dyn Mapper>> {
        vec![
            Box::new(TeamFileMapper::build(self.project.clone())),
            Box::new(TeamGlobMapper::build(self.project.clone())),
            Box::new(DirectoryMapper::build(self.project.clone())),
            Box::new(RubyPackageMapper::build(self.project.clone())),
            Box::new(JavascriptPackageMapper::build(self.project.clone())),
            Box::new(TeamYmlMapper::build(self.project.clone())),
            Box::new(TeamGemMapper::build(self.project.clone())),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common_test::tests::{TestConfig, TestProjectFile, build_ownership, build_ownership_with_all_mappers};
    use indoc::indoc;
    use tempfile::tempdir;

    fn build_annotation_over_directory(allow_override: bool) -> Result<Ownership, Box<dyn Error>> {
        let base_yml = indoc! {"
            ---
            owned_globs:
              - \"app/**/*.rb\"
            unowned_globs:
              - config/code_ownership.yml
            team_file_glob:
              - config/teams/**/*.yml
        "};
        let yml = if allow_override {
            format!("{base_yml}allow_ownership_override: true\n")
        } else {
            base_yml.to_owned()
        };

        let temp_dir = tempdir()?;
        let mut test_config = TestConfig::new(
            temp_dir.path().to_path_buf(),
            vec![
                TestProjectFile {
                    relative_path: "app/foo/.codeowner".to_owned(),
                    content: "Bar\n".to_owned(),
                },
                TestProjectFile {
                    relative_path: "app/foo/thing.rb".to_owned(),
                    content: "# @team Foo\nclass Thing\nend\n".to_owned(),
                },
            ],
        );
        test_config.code_ownership_config_yml = yml;
        // The generated CODEOWNERS would list both owners and make the stale-file
        // check noisy; this test targets the multiple-owners rule only.
        test_config.generate_codeowners = false;
        build_ownership(test_config)
    }

    #[test]
    fn test_annotation_over_directory_errors_without_override() -> Result<(), Box<dyn Error>> {
        let ownership = build_annotation_over_directory(false)?;
        let errors = ownership.validate().expect_err("expected validation to fail when override is off");
        assert!(
            errors.to_string().contains("multiple"),
            "expected a multiple-owners error, got: {errors}"
        );
        Ok(())
    }

    #[test]
    fn test_annotation_over_directory_ok_with_override() -> Result<(), Box<dyn Error>> {
        let ownership = build_annotation_over_directory(true)?;
        let file_owners = ownership.for_file(Path::new("app/foo/thing.rb")).unwrap();
        let top = file_owners
            .iter()
            .min_by_key(|owner| owner.sources.iter().map(Source::priority).min().unwrap_or(u8::MAX))
            .expect("expected at least one owner");
        assert_eq!(top.team.name, "Foo", "annotation should win over the directory owner");

        let validation_errors = ownership.validate().err();
        let has_multiple_owner_error = validation_errors
            .map(|errors| errors.to_string().contains("multiple"))
            .unwrap_or(false);
        assert!(!has_multiple_owner_error, "override on should not raise a multiple-owners error");
        Ok(())
    }

    #[test]
    fn test_for_file_owner() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_all_mappers()?;
        let file_owners = ownership.for_file(Path::new("app/consumers/directory_owned.rb")).unwrap();
        assert_eq!(file_owners.len(), 1);
        assert_eq!(file_owners[0].team.name, "Bar");
        assert_eq!(file_owners[0].team_config_file_path, "config/teams/bar.yml");
        Ok(())
    }

    #[test]
    fn test_for_file_no_owner() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_all_mappers()?;
        let file_owners = ownership.for_file(Path::new("app/madeup/foo.rb")).unwrap();
        assert_eq!(file_owners.len(), 0);
        Ok(())
    }

    #[test]
    fn test_for_team() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_all_mappers()?;
        let team_ownership = ownership.for_team("Bar");
        assert!(team_ownership.is_ok());
        Ok(())
    }

    #[test]
    fn test_for_team_not_found() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_all_mappers()?;
        let team_ownership = ownership.for_team("Nope");
        assert!(team_ownership.is_err(), "Team not found");
        Ok(())
    }

    #[test]
    fn test_ownership_entries_groups_by_mapper() -> Result<(), Box<dyn Error>> {
        let ownership = build_ownership_with_all_mappers()?;
        let groups = ownership.ownership_entries();

        // Every mapper is represented, in mapper order.
        let names: Vec<String> = groups.iter().map(|group| group.mapper.clone()).collect();
        assert!(names.contains(&"Owner in .codeowner".to_string()));

        // The directory mapper reports its owner as a `/**/**` glob with the team name.
        let directory_group = groups
            .iter()
            .find(|group| group.mapper == "Owner in .codeowner")
            .expect("expected a directory mapper group");
        let consumers = directory_group
            .entries
            .iter()
            .find(|entry| entry.path == "app/consumers/**/**")
            .expect("expected the app/consumers directory entry");
        assert_eq!(consumers.team_name, "Bar");
        Ok(())
    }
}
