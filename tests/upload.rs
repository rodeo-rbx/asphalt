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

#[test]
fn create_requires_a_creator() {
    project_with_file()
        .run()
        .args(["upload", FILE])
        .assert()
        .failure()
        .stderr(contains("--creator-type"));
}

#[test]
fn creator_type_and_id_go_together() {
    project_with_file()
        .run()
        .args([
            "upload",
            FILE,
            "--asset-id",
            "123",
            "--creator-type",
            "group",
        ])
        .assert()
        .failure()
        .stderr(contains("--creator-id"));
}

#[test]
fn create_json_reports_the_first_version() {
    project_with_file()
        .run()
        .args([
            "upload",
            FILE,
            "--creator-type",
            "user",
            "--creator-id",
            "1",
            "--json",
        ])
        .assert()
        .success()
        .stdout(is_match(r#"^\{"asset_id":\d+,"version":1\}\n$"#).unwrap());
}

#[test]
fn update_needs_no_creator_and_prints_id_and_version() {
    project_with_file()
        .run()
        .args(["upload", FILE, "--asset-id", "123"])
        .assert()
        .success()
        .stdout("{\"asset_id\":123,\"version\":2}\n")
        .stderr("");
}

#[test]
fn update_with_json_prints_the_same() {
    project_with_file()
        .run()
        .args(["upload", FILE, "--asset-id", "123", "--json"])
        .assert()
        .success()
        .stdout("{\"asset_id\":123,\"version\":2}\n");
}

#[test]
fn update_link_points_at_the_same_asset_and_logs_the_version() {
    project_with_file()
        .run()
        .args(["upload", FILE, "--asset-id", "123", "--link"])
        .assert()
        .success()
        .stdout("https://create.roblox.com/store/asset/123\n")
        .stderr(contains("Asset 123 is now at version 2"));
}

#[test]
fn create_still_prints_the_bare_asset_id() {
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
fn link_and_json_conflict() {
    project_with_file()
        .run()
        .args(["upload", FILE, "--asset-id", "123", "--link", "--json"])
        .assert()
        .failure();
}

#[test]
fn expected_price_needs_a_creator() {
    project_with_file()
        .run()
        .args(["upload", FILE, "--asset-id", "123", "--expected-price", "5"])
        .assert()
        .failure()
        .stderr(contains("--creator-type"));
}
