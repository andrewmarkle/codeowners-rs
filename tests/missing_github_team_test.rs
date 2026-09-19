use predicates::prelude::*;
use std::error::Error;

mod common;
use common::OutputStream;
use common::run_codeowners;

// A team file without a `github` block is valid. GitLab teams carry only a
// `gitlab:` block, so the parser must accept a team file that has no `github`
// key. The fixture's bad_team.yml has `name` only; loading it must not raise a
// parse error. The command succeeds and stderr has no "missing field `github`".
#[test]
fn test_team_file_without_github_block_is_accepted() -> Result<(), Box<dyn Error>> {
    run_codeowners(
        "missing_github_team",
        &["for-file", "--from-codeowners", "gems/pets/dog.rb"],
        true, // command succeeds; the github-less team file parses cleanly
        OutputStream::Stderr,
        predicate::str::contains("missing field `github`").not(),
    )
}
