use common::Project;
use predicates::str::{contains, is_match};

mod common;

const FILE: &str = "input/test1.png";

fn project_with_file() -> Project {
    let project = Project::new();
    project.add_file("test1.png");
    project
}

#[test]
fn create_reads_api_key_from_environment() {
    // `Project::run` sets ASPHALT_API_KEY and no --api-key is passed.
    project_with_file()
        .run()
        .args([
            "upload",
            FILE,
            "--creator-type",
            "user",
            "--creator-id",
            "1",
        ])
        .assert()
        .success()
        .stdout(is_match(r"^\d+\n$").unwrap());
}

#[test]
fn missing_api_key_fails() {
    project_with_file()
        .run()
        .env_remove("ASPHALT_API_KEY")
        .args([
            "upload",
            FILE,
            "--creator-type",
            "user",
            "--creator-id",
            "1",
        ])
        .assert()
        .failure()
        .stderr(contains("API key"));
}
